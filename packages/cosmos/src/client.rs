pub(crate) mod node;
mod node_chooser;
mod pool;
pub(crate) mod query;

use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use chrono::{DateTime, TimeZone, Utc};
use cosmos_sdk_proto::{
    cosmos::{
        auth::v1beta1::{BaseAccount, QueryAccountRequest},
        bank::v1beta1::QueryAllBalancesRequest,
        base::{
            abci::v1beta1::TxResponse,
            query::v1beta1::PageRequest,
            tendermint::v1beta1::{GetBlockByHeightRequest, GetLatestBlockRequest},
            v1beta1::Coin,
        },
        tx::v1beta1::{
            AuthInfo, BroadcastMode, BroadcastTxRequest, BroadcastTxResponse, Fee, GetTxRequest,
            GetTxResponse, GetTxsEventRequest, ModeInfo, OrderBy, SignDoc, SignerInfo,
            SimulateRequest, SimulateResponse, Tx, TxBody,
        },
    },
    cosmwasm::wasm::v1::QueryCodeRequest,
    traits::Message,
};
use parking_lot::{Mutex, RwLock};
use tokio::{sync::mpsc::Receiver, task::JoinSet, time::Instant};
use tonic::{
    metadata::{Ascii, MetadataKey, MetadataValue},
    service::Interceptor,
    Status,
};

use crate::{
    address::HasAddressHrp,
    error::{
        Action, BuilderError, ConnectionError, CosmosSdkError, FirstBlockAfterError,
        NodeHealthReport, QueryError, QueryErrorCategory, QueryErrorDetails,
    },
    gas_multiplier::{GasMultiplier, GasMultiplierConfig},
    gas_price::{CurrentGasPrice, DEFAULT_GAS_PRICE},
    osmosis::ChainPausedStatus,
    wallet::WalletPublicKey,
    Address, CosmosBuilder, DynamicGasMultiplier, Error, HasAddress, TxBuilder,
};

use self::{node::Node, node_chooser::QueryResult, pool::Pool, query::GrpcRequest};

use super::Wallet;

/// A connection to a gRPC endpoint to communicate with a Cosmos chain.
///
/// Behind the scenes, this uses a [Pool] of connections. Cloning this value is
/// cheap and recommended, it will encourage connection sharing.
///
/// See [CosmosBuilder] and [crate::CosmosNetwork] for common methods of
/// building a [Cosmos].
#[derive(Clone)]
pub struct Cosmos {
    pool: Pool,
    height: Option<u64>,
    pub(crate) chain_paused_status: ChainPausedStatus,
    gas_multiplier: GasMultiplier,
    /// Maximum gas price
    pub(crate) max_price: f64,
    tracking: Arc<Tracking>,
}

struct Tracking {
    block_height: Mutex<BlockHeightTracking>,
    simulate_sequences: RwLock<HashMap<Address, SequenceInformation>>,
    broadcast_sequences: RwLock<HashMap<Address, SequenceInformation>>,
}

pub(crate) struct WeakCosmos {
    pool: Pool,
    height: Option<u64>,
    tracking: Weak<Tracking>,
    chain_paused_status: ChainPausedStatus,
    gas_multiplier: GasMultiplier,
    max_price: f64,
}

/// Type encapsulating both the [TxResponse] as well the actual [Tx]
/// which will be helpful in the inspection of fees etc.
pub struct CosmosTxResponse {
    /// Transaction response
    pub response: TxResponse,
    /// Transaction representing it's body, signature and other
    /// information.
    pub tx: Tx,
}

impl From<&Cosmos> for WeakCosmos {
    fn from(
        Cosmos {
            pool,
            height,
            tracking,
            chain_paused_status,
            gas_multiplier,
            max_price,
        }: &Cosmos,
    ) -> Self {
        WeakCosmos {
            pool: pool.clone(),
            height: *height,
            tracking: Arc::downgrade(tracking),
            chain_paused_status: chain_paused_status.clone(),
            gas_multiplier: gas_multiplier.clone(),
            max_price: *max_price,
        }
    }
}

impl WeakCosmos {
    pub(crate) fn upgrade(&self) -> Option<Cosmos> {
        let WeakCosmos {
            pool,
            height,
            tracking,
            chain_paused_status,
            gas_multiplier,
            max_price,
        } = self;
        tracking.upgrade().map(|tracking| Cosmos {
            pool: pool.clone(),
            height: *height,
            tracking,
            chain_paused_status: chain_paused_status.clone(),
            gas_multiplier: gas_multiplier.clone(),
            max_price: *max_price,
        })
    }
}

struct BlockHeightTracking {
    /// Local time when this block height was observed
    when: Instant,
    /// Height that was seen
    height: i64,
}

impl std::fmt::Debug for Cosmos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cosmos")
            .field("builder", &self.pool.builder)
            .field("height", &self.height)
            .finish()
    }
}

pub(crate) struct PerformQueryBuilder<'a, Request> {
    cosmos: &'a Cosmos,
    req: Request,
    action: Action,
    should_retry: bool,
    all_nodes: bool,
}

struct PerformQueryError {
    details: QueryErrorDetails,
    grpc_url: Arc<String>,
}

struct PerformQueryResponse<'a, Request: GrpcRequest> {
    cosmos: &'a Cosmos,
    rx: Receiver<Result<PerformQueryWrapper<Request::Response>, PerformQueryError>>,
    set: JoinSet<()>,
    is_all_nodes: bool,
    action: Action,
}

impl<Request: GrpcRequest> Drop for PerformQueryResponse<'_, Request> {
    fn drop(&mut self) {
        // If we were doing an all-nodes broadcast, let remaining tasks
        // complete in case the successful broadcast went to a node
        // where the transactions aren't being shared to other mempools
        // correctly.
        if !self.is_all_nodes {
            self.set.abort_all();
        }
    }
}

impl<Request: GrpcRequest> PerformQueryResponse<'_, Request> {
    fn make_error(&self, query: QueryErrorDetails, grpc_url: Arc<String>) -> QueryError {
        QueryError {
            action: self.action.clone(),
            builder: self.cosmos.pool.builder.clone(),
            height: self.cosmos.height,
            query,
            grpc_url,
            node_health: self.cosmos.pool.node_chooser.health_report(),
        }
    }
}

impl<Request: GrpcRequest> PerformQueryBuilder<'_, Request> {
    async fn run_with<T, E, Mapper>(self, mapper: Mapper) -> Result<T, E>
    where
        Mapper: Fn(
            &PerformQueryResponse<Request>,
            Result<PerformQueryWrapper<Request::Response>, QueryError>,
        ) -> Result<T, E>,
        E: From<QueryError> + std::fmt::Display,
    {
        let mut first_error = None;
        let mut pqr = run_query(self).await?;
        loop {
            let err = match pqr.rx.recv().await {
                None => break,
                Some(res) => {
                    let res = res.map_err(|PerformQueryError { details, grpc_url }| {
                        pqr.make_error(details, grpc_url)
                    });
                    match mapper(&pqr, res) {
                        Ok(success) => return Ok(success),
                        Err(err) => err,
                    }
                }
            };
            if first_error.is_some() {
                tracing::warn!("Extra error while looking for success response from nodes: {err}");
            } else {
                first_error = Some(err);
            }
        }

        Err(first_error.unwrap_or_else(|| {
            pqr.make_error(
                QueryErrorDetails::ConnectionError(ConnectionError::NoHealthyFound),
                pqr.cosmos.get_cosmos_builder().grpc_url_arc().clone(),
            )
            .into()
        }))
    }

    pub(crate) async fn run(self) -> Result<PerformQueryWrapper<Request::Response>, QueryError> {
        self.run_with(|_pqr, res| res).await
    }

    pub(crate) fn no_retry(mut self) -> Self {
        self.should_retry = false;
        self
    }

    fn all_nodes(mut self) -> Self {
        self.all_nodes = true;
        self
    }
}

impl PerformQueryBuilder<'_, BroadcastTxRequest> {
    async fn run_broadcast(
        self,
        skip_code_check: bool,
    ) -> Result<(Arc<String>, TxResponse), crate::Error> {
        self.run_with(|pqr, res| {
            let res = res?;
            let grpc_url = res.grpc_url;
            let res = res.tonic.into_inner().tx_response.ok_or_else(|| {
                crate::Error::InvalidChainResponse {
                    message: "Missing inner tx_response".to_owned(),
                    action: pqr.action.clone().into(),
                }
            })?;

            // Check if the transaction was successfully broadcast. We have three
            // ways for this to "succeed":
            //
            // 1. We've decided to skip checking the code entirely.
            // 2. The broadcast succeeded (status 0)
            // 3. The broadcast failed with code 19, meaning "already in mempool"
            //
            // Our assumption with (3) is that we don't care about reporting if
            // the tx is already in the pool, we just want to wait for it to be
            // included in a block. Note that it's common for code 19 to occur
            // when using all-node broadcasting.
            if !(skip_code_check
                || res.code == 0
                || CosmosSdkError::from_code(res.code, &res.codespace).is_successful_broadcast())
            {
                Err(crate::Error::TransactionFailed {
                    code: CosmosSdkError::from_code(res.code, &res.codespace),
                    txhash: res.txhash.clone(),
                    raw_log: res.raw_log,
                    action: pqr.action.clone().into(),
                    grpc_url,
                    stage: crate::error::TransactionStage::Broadcast,
                })
            } else {
                Ok((grpc_url, res))
            }
        })
        .await
    }
}

pub(crate) struct PerformQueryWrapper<Res> {
    pub(crate) grpc_url: Arc<String>,
    pub(crate) tonic: tonic::Response<Res>,
}
impl<Res> PerformQueryWrapper<Res> {
    pub(crate) fn into_inner(self) -> Res {
        self.tonic.into_inner()
    }
}

impl Cosmos {
    async fn get_and_update_simulation_sequence(
        &self,
        address: Address,
    ) -> Result<BaseAccount, Error> {
        let sequence = {
            let guard = self.tracking.simulate_sequences.read();
            let result = guard.get(&address);
            result.cloned()
        };
        let mut base_account = self.get_base_account(address).await?;
        if let Some(SequenceInformation {
            sequence,
            timestamp,
        }) = sequence
        {
            let diff = Instant::now().duration_since(timestamp);
            if diff.as_secs() <= 30 {
                let max_sequence = std::cmp::max(sequence, base_account.sequence);
                if max_sequence != sequence {
                    let sequence_info = SequenceInformation {
                        sequence: max_sequence,
                        timestamp: Instant::now(),
                    };
                    {
                        let mut seq_info = self.tracking.simulate_sequences.write();
                        let seq_info = seq_info
                            .entry(address)
                            .or_insert_with(|| sequence_info.clone());
                        *seq_info = sequence_info;
                    }
                }
                base_account.sequence = max_sequence;
                return Ok(base_account);
            }
        }
        let mut seq_info = self.tracking.simulate_sequences.write();
        let sequence_info = SequenceInformation {
            sequence: base_account.sequence,
            timestamp: Instant::now(),
        };
        let seq_info = seq_info
            .entry(address)
            .or_insert_with(|| sequence_info.clone());
        *seq_info = sequence_info;
        Ok(base_account)
    }

    async fn update_broadcast_sequence(
        &self,
        address: Address,
        tx: &Tx,
        hash: &str,
    ) -> Result<(), Error> {
        let auth_info = &tx.auth_info;
        if let Some(auth_info) = auth_info {
            // This only works since we allow a single signer per
            // transaction. This needs to be updated to check with
            // public key when multiple signers exist.
            let sequence = &auth_info
                .signer_infos
                .iter()
                .map(|item| item.sequence)
                .max();
            match sequence {
                Some(sequence) => {
                    let mut sequences = self.tracking.broadcast_sequences.write();
                    sequences
                        .entry(address)
                        .and_modify(|item| item.sequence = *sequence);
                }
                None => {
                    tracing::warn!("No sequence number found in Tx {hash} from signer_infos");
                }
            }
        } else {
            tracing::warn!("No sequence number found in Tx {hash} from auth_info");
        }

        Ok(())
    }

    async fn get_and_update_broadcast_sequence(
        &self,
        address: Address,
    ) -> Result<BaseAccount, Error> {
        let sequence = {
            let guard = self.tracking.broadcast_sequences.read();
            let result = guard.get(&address);
            result.cloned()
        };
        let mut base_account = self.get_base_account(address).await?;
        if let Some(SequenceInformation {
            sequence,
            timestamp,
        }) = sequence
        {
            let diff = Instant::now().duration_since(timestamp);
            if diff.as_secs() <= 30 {
                let max_sequence = std::cmp::max(sequence, base_account.sequence);
                if max_sequence != sequence {
                    let sequence_info = SequenceInformation {
                        sequence: max_sequence,
                        timestamp: Instant::now(),
                    };
                    {
                        let mut seq_info = self.tracking.broadcast_sequences.write();
                        let seq_info = seq_info
                            .entry(address)
                            .or_insert_with(|| sequence_info.clone());
                        *seq_info = sequence_info;
                    }
                }
                base_account.sequence = max_sequence;
                return Ok(base_account);
            }
        }
        let mut seq_info = self.tracking.broadcast_sequences.write();
        let sequence_info = SequenceInformation {
            sequence: base_account.sequence,
            timestamp: Instant::now(),
        };
        let seq_info = seq_info
            .entry(address)
            .or_insert_with(|| sequence_info.clone());
        *seq_info = sequence_info;
        Ok(base_account)
    }

    pub(crate) fn perform_query<Request: GrpcRequest>(
        &self,
        req: Request,
        action: Action,
    ) -> PerformQueryBuilder<Request> {
        PerformQueryBuilder {
            cosmos: self,
            req,
            action,
            should_retry: true,
            all_nodes: false,
        }
    }
}

async fn run_query<Request: GrpcRequest>(
    PerformQueryBuilder {
        cosmos,
        req,
        action,
        should_retry,
        all_nodes,
    }: PerformQueryBuilder<'_, Request>,
) -> Result<PerformQueryResponse<'_, Request>, QueryError> {
    // This function is responsible for running queries against blockchain nodes.
    // There are two primary ways of operating:
    //
    // All nodes: this is used when broadcasting transactions. The idea is that we
    // want to broadcast to _all_ non-blocked nodes, since sometimes some nodes are
    // unable to rebroadcast transactions from their mempool over P2P. In this case,
    // we want to broadcast to all nodes immediately, return from this function as
    // soon as the first success comes through, and let broadcasts to other nodes
    // continue in the background.
    //
    // Regular: for everything else, we don't want to spam all nodes with every
    // request. Instead, we get a priority list of the healthiest nodes and try them
    // in order. We delay each successive node by a configurable amount of time to
    // allow the earlier nodes to complete. We want to return from this function as
    // soon as the first success comes through, but we want to cancel all remaining
    // work at that point.

    // Set up channels for the individual workers to send back either success or
    // error results. We keep separate channels, since we may want to optimistically
    // exit on an early success, but likely want to wait for all nodes on failure.

    // Grab some config values.
    let all_nodes_broadcast = all_nodes && cosmos.get_cosmos_builder().get_all_nodes_broadcast();
    let delay = cosmos.get_cosmos_builder().get_delay_before_fallback();
    let total_attempts = cosmos.pool.builder.query_retries();

    // Get the set of nodes we should run against.
    let nodes = if all_nodes_broadcast {
        cosmos
            .pool
            .all_nodes()
            .filter(|node| match node.node_health_level() {
                crate::error::NodeHealthLevel::Unblocked { error_count: _ } => true,
                crate::error::NodeHealthLevel::Blocked => false,
            })
            .cloned()
            .collect()
    } else {
        cosmos.pool.node_chooser.choose_nodes()
    };

    if cosmos.pool.builder.get_log_requests() {
        tracing::info!("{action}");
    }

    // Prepare for parallel execution
    let mut set = JoinSet::new();
    let (tx, rx) = tokio::sync::mpsc::channel(nodes.len().max(4));

    for (node_idx, node) in nodes.into_iter().enumerate() {
        // Cloning for passing into the async move
        let tx = tx.clone();
        let action = action.clone();
        let req = req.clone();
        let cosmos = cosmos.clone();
        set.spawn(async move {
            if node_idx != 0 {
                tokio::time::sleep(delay).await;
            }
                for attempt in 1..=total_attempts {
                    let _permit = cosmos.pool.get_node_permit().await;
                    match cosmos.perform_query_inner(req.clone(), &node).await {
                        Ok(tonic) => {
                            node.log_query_result(QueryResult::Success);
                            tx
                                .try_send(Ok(PerformQueryWrapper {
                                    grpc_url: node.grpc_url().clone(),
                                    tonic,
                                }))
                                .ok();
                            break;
                        }
                        Err((err, can_retry)) => {
                            tracing::debug!("Error performing a query. Attempt {attempt} of {total_attempts}. can_retry={can_retry}. should_retry={should_retry}. {err}");
                            node.log_query_result(if can_retry {
                                QueryResult::NetworkError {
                                    err: err.clone(),
                                    action: action.clone(),
                                }
                            } else {
                                QueryResult::OtherError
                            });
                            tx.try_send(Err(PerformQueryError { details: err, grpc_url: node.grpc_url().clone() })).ok();
                            if !can_retry || !should_retry {
                                break;
                            }
                        }
                    }
                }
            });
    }

    Ok(PerformQueryResponse {
        cosmos,
        rx,
        set,
        is_all_nodes: all_nodes_broadcast,
        action,
    })
}

impl Cosmos {
    /// Error return: the details itself, and whether a retry can be attempted.
    async fn perform_query_inner<Request: GrpcRequest>(
        &self,
        req: Request,
        cosmos_inner: &Node,
    ) -> Result<tonic::Response<Request::Response>, (QueryErrorDetails, bool)> {
        let duration =
            tokio::time::Duration::from_secs(self.pool.builder.query_timeout_seconds().into());
        let mut req = tonic::Request::new(req.clone());
        if let Some(height) = self.height {
            // https://docs.cosmos.network/v0.47/run-node/interact-node#query-for-historical-state-using-rest
            let metadata = req.metadata_mut();
            metadata.insert("x-cosmos-block-height", height.into());
        }
        let res = tokio::time::timeout(duration, GrpcRequest::perform(req, cosmos_inner)).await;
        match res {
            Ok(Ok(res)) => {
                self.check_block_height(
                    res.metadata().get("x-cosmos-block-height"),
                    cosmos_inner.grpc_url(),
                )?;
                Ok(res)
            }
            Ok(Err(status)) => {
                let err = QueryErrorDetails::from_tonic_status(status);
                let can_retry = match err.error_category() {
                    QueryErrorCategory::NetworkIssue => {
                        cosmos_inner
                            .set_broken(|grpc_url| ConnectionError::QueryFailed { grpc_url }, &err);
                        true
                    }
                    QueryErrorCategory::ConnectionIsFine => false,
                    QueryErrorCategory::Unsure => {
                        // Not enough info from the error to determine what went
                        // wrong. Send a basic request that should always
                        // succeed to determine if it's a network issue or not.
                        match GrpcRequest::perform(
                            tonic::Request::new(GetLatestBlockRequest {}),
                            cosmos_inner,
                        )
                        .await
                        {
                            Ok(_) => {
                                // OK, connection looks fine, don't bother retrying
                                false
                            }
                            Err(status) => {
                                // Something went wrong. Don't even bother
                                // looking at _what_ went wrong, just kill this
                                // connection and retry.
                                cosmos_inner.set_broken(
                                    |grpc_url| ConnectionError::SanityCheckFailed {
                                        grpc_url,
                                        source: status,
                                    },
                                    &err,
                                );
                                true
                            }
                        }
                    }
                };

                Err((err, can_retry))
            }
            Err(_) => {
                let err = QueryErrorDetails::QueryTimeout(duration);
                cosmos_inner
                    .set_broken(|grpc_url| ConnectionError::TimeoutQuery { grpc_url }, &err);
                Err((err, true))
            }
        }
    }

    /// Get the [CosmosBuilder] used to construct this connection.
    pub fn get_cosmos_builder(&self) -> &Arc<CosmosBuilder> {
        &self.pool.builder
    }

    fn check_block_height(
        &self,
        new_height: Option<&tonic::metadata::MetadataValue<tonic::metadata::Ascii>>,
        grpc_url: &Arc<String>,
    ) -> Result<(), (QueryErrorDetails, bool)> {
        if self.height.is_some() {
            // Don't do a height check, we're specifically querying historical data.
            return Ok(());
        }
        // If the chain is paused, don't do a block height check either
        if self.chain_paused_status.is_paused() {
            return Ok(());
        }

        let new_height = match new_height {
            Some(header_value) => header_value,
            None => {
                tracing::warn!(
                    "No x-cosmos-block-height response header found on request to {grpc_url}"
                );
                return Ok(());
            }
        };
        let new_height = match new_height.to_str() {
            Ok(new_height) => {
                tracing::debug!("x-cosmos-block-height value is: {new_height}");
                new_height
            }
            Err(err) => {
                tracing::warn!("x-cosmos-block-height response header from {grpc_url} does not contain textual data: {err}");
                return Ok(());
            }
        };
        let new_height: i64 = match new_height.parse() {
            Ok(new_height) => new_height,
            Err(err) => {
                tracing::warn!("x-cosmos-block-height response header from {grpc_url} is {new_height}, could not parse as i64: {err}");
                return Ok(());
            }
        };
        let now = Instant::now();

        let mut guard = self.tracking.block_height.lock();

        let BlockHeightTracking {
            when: prev,
            height: old_height,
        } = *guard;

        // We're moving forward so update the tracking and move on.
        if new_height > old_height {
            *guard = BlockHeightTracking {
                when: now,
                height: new_height,
            };
            return Ok(());
        }

        // Check if we're too many blocks lagging.
        if old_height - new_height > i64::from(self.get_cosmos_builder().block_lag_allowed()) {
            return Err((
                QueryErrorDetails::BlocksLagDetected {
                    old_height,
                    new_height,
                    block_lag_allowed: self.get_cosmos_builder().block_lag_allowed(),
                },
                true,
            ));
        }

        // And now see if it's been too long since we've seen any new blocks.
        let age = match now.checked_duration_since(prev) {
            Some(age) => age,
            None => {
                tracing::warn!("Error subtracting two Instants: {now:?} - {prev:?}");
                return Ok(());
            }
        };

        if age > self.get_cosmos_builder().latest_block_age_allowed() {
            return Err((
                QueryErrorDetails::NoNewBlockFound {
                    age,
                    age_allowed: self.get_cosmos_builder().latest_block_age_allowed(),
                    old_height,
                    new_height,
                },
                true,
            ));
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct CosmosInterceptor(Arc<[(MetadataKey<Ascii>, MetadataValue<Ascii>)]>);

impl Interceptor for CosmosInterceptor {
    fn call(&mut self, mut request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        let meta = request.metadata_mut();
        for (key, value) in self.0.iter() {
            meta.insert(key, value.clone());
        }
        Ok(request)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SequenceInformation {
    sequence: u64,
    timestamp: Instant,
}

impl CosmosBuilder {
    /// Create a new [Cosmos] but do not perform any sanity checks.
    ///
    /// Can fail if parsing the gRPC URLs fails.
    pub fn build(self) -> Result<Cosmos, BuilderError> {
        let builder = Arc::new(self);
        let chain_paused_status = builder.chain_paused_method.into();
        let gas_multiplier = builder.build_gas_multiplier();
        let max_price = builder.get_init_max_gas_price();
        let cosmos = Cosmos {
            pool: Pool::new(builder)?,
            height: None,
            tracking: Arc::new(Tracking {
                block_height: Mutex::new(BlockHeightTracking {
                    when: Instant::now(),
                    height: 0,
                }),
                simulate_sequences: RwLock::new(HashMap::new()),
                broadcast_sequences: RwLock::new(HashMap::new()),
            }),
            chain_paused_status,
            gas_multiplier,
            max_price,
        };
        cosmos.launch_chain_paused_tracker();

        Ok(cosmos)
    }
}

impl Cosmos {
    /// Return a modified version of this [Cosmos] that queries at the given height.
    pub fn at_height(mut self, height: Option<u64>) -> Self {
        self.height = height;
        self
    }

    /// Return a modified version of this [Cosmos] that sets the maximum gas price to this value.
    ///
    /// Only has an impact on Osmosis mainnet.
    pub fn with_max_gas_price(mut self, max_price: f64) -> Self {
        self.max_price = max_price;
        self
    }

    /// Return a modified version of this [Cosmos] with a separate dynamic gas value.
    ///
    /// This is useful for being able to share connections across an application, but allow different pieces of the application to calculate the gas multiplier separately. For example, send-coin heavy workloads will likely need a higher multiplier.
    pub fn with_dynamic_gas(mut self, dynamic: DynamicGasMultiplier) -> Self {
        self.gas_multiplier = GasMultiplierConfig::Dynamic(dynamic).build();
        self
    }

    /// Return the currently used gas multiplier.
    pub fn get_current_gas_multiplier(&self) -> f64 {
        self.gas_multiplier.get_current()
    }

    /// Are we using a dynamic gas multiplier?
    pub fn is_gas_multiplier_dynamic(&self) -> bool {
        match self.gas_multiplier {
            GasMultiplier::Static(_) => false,
            GasMultiplier::Dynamic(_) => true,
        }
    }

    /// Get the base account information for the given address.
    pub async fn get_base_account(&self, address: Address) -> Result<BaseAccount, crate::Error> {
        let action = Action::GetBaseAccount(address);
        let res = self
            .perform_query(
                QueryAccountRequest {
                    address: address.get_address_string(),
                },
                action.clone(),
            )
            .run()
            .await?
            .into_inner();

        let base_account = if self.get_address_hrp().as_str() == "inj" {
            let eth_account: crate::injective::EthAccount = prost::Message::decode(
                res.account
                    .ok_or_else(|| crate::Error::InvalidChainResponse {
                        message: "no eth account found".to_owned(),
                        action: action.clone().into(),
                    })?
                    .value
                    .as_ref(),
            )
            .map_err(|source| crate::Error::InvalidChainResponse {
                message: format!("Unable to parse eth_account: {source}"),
                action: action.clone().into(),
            })?;
            eth_account
                .base_account
                .ok_or_else(|| crate::Error::InvalidChainResponse {
                    message: "no base account found".to_owned(),
                    action: action.clone().into(),
                })?
        } else {
            prost::Message::decode(
                res.account
                    .ok_or_else(|| crate::Error::InvalidChainResponse {
                        message: "no account found".to_owned(),
                        action: action.clone().into(),
                    })?
                    .value
                    .as_ref(),
            )
            .map_err(|source| crate::Error::InvalidChainResponse {
                message: format!("Unable to parse account: {source}"),
                action: action.into(),
            })?
        };
        Ok(base_account)
    }

    /// Get the coin balances for the given address.
    pub async fn all_balances(&self, address: Address) -> Result<Vec<Coin>, crate::Error> {
        let mut coins = Vec::new();
        let mut pagination = None;
        loop {
            let mut res = self
                .perform_query(
                    QueryAllBalancesRequest {
                        address: address.get_address_string(),
                        pagination: pagination.take(),
                        resolve_denom: false,
                    },
                    Action::QueryAllBalances(address),
                )
                .run()
                .await?
                .into_inner();
            coins.append(&mut res.balances);
            match res.pagination {
                Some(x) if !x.next_key.is_empty() => {
                    pagination = Some(PageRequest {
                        key: x.next_key,
                        offset: 0,
                        limit: 0,
                        count_total: false,
                        reverse: false,
                    })
                }
                _ => break Ok(coins),
            }
        }
    }

    pub(crate) async fn code_info(&self, code_id: u64) -> Result<Vec<u8>, crate::Error> {
        let res = self
            .perform_query(QueryCodeRequest { code_id }, Action::CodeInfo(code_id))
            .run()
            .await?;
        Ok(res.into_inner().data)
    }

    fn txres_to_tuple(
        txres: GetTxResponse,
        action: Action,
    ) -> Result<(TxBody, AuthInfo, TxResponse), crate::Error> {
        let tx = txres.tx.ok_or_else(|| crate::Error::InvalidChainResponse {
            message: "Missing tx field".to_owned(),
            action: action.clone().into(),
        })?;
        let txbody = tx.body.ok_or_else(|| crate::Error::InvalidChainResponse {
            message: "Missing tx.body field".to_owned(),
            action: action.clone().into(),
        })?;
        let auth_info = tx
            .auth_info
            .ok_or_else(|| crate::Error::InvalidChainResponse {
                message: "Missing tx.auth_info field".to_owned(),
                action: action.clone().into(),
            })?;
        let txres = txres
            .tx_response
            .ok_or_else(|| crate::Error::InvalidChainResponse {
                message: "Missing tx_response field".to_owned(),
                action: action.clone().into(),
            })?;
        Ok((txbody, auth_info, txres))
    }

    /// Get a transaction, failing immediately if not present
    ///
    /// This will follow normal fallback rules for other queries. You may want
    /// to try out [Self::get_transaction_with_fallbacks].
    pub async fn get_transaction_body(
        &self,
        txhash: impl Into<String>,
    ) -> Result<(TxBody, AuthInfo, TxResponse), crate::Error> {
        let txhash = txhash.into();
        let action = Action::GetTransactionBody(txhash.clone());
        let txres = self
            .perform_query(
                GetTxRequest {
                    hash: txhash.clone(),
                },
                action.clone(),
            )
            .run()
            .await?
            .into_inner();
        Self::txres_to_tuple(txres, action)
    }

    /// Get a transaction with more aggressive fallback usage.
    ///
    /// This is intended to help indexers. A common failure mode in Cosmos is a
    /// single missing transaction on some nodes. This method will first try to
    /// get the transaction following normal fallback rules, and if that fails,
    /// will iterate through all fallbacks.
    pub async fn get_transaction_with_fallbacks(
        &self,
        txhash: impl Into<String>,
    ) -> Result<(TxBody, AuthInfo, TxResponse), crate::Error> {
        let txhash = txhash.into();
        let action = Action::GetTransactionBody(txhash.clone());
        let res = self
            .perform_query(
                GetTxRequest {
                    hash: txhash.clone(),
                },
                action.clone(),
            )
            .no_retry()
            .run()
            .await;
        match res {
            Ok(txres) => Self::txres_to_tuple(txres.into_inner(), action),
            Err(e) => {
                for node in self.pool.node_chooser.all_nodes() {
                    let _permit = self.pool.get_node_permit().await;
                    if let Ok(txres) = self
                        .perform_query_inner(
                            GetTxRequest {
                                hash: txhash.clone(),
                            },
                            node,
                        )
                        .await
                    {
                        return Self::txres_to_tuple(txres.into_inner(), action);
                    }
                }
                Err(e.into())
            }
        }
    }

    /// Wait for a transaction to land on-chain using a busy loop.
    ///
    /// This is most useful after broadcasting a transaction to wait for it to land.
    pub async fn wait_for_transaction(
        &self,
        txhash: impl Into<String>,
    ) -> Result<(TxBody, AuthInfo, TxResponse), crate::Error> {
        self.wait_for_transaction_with_action(txhash, None).await
    }

    async fn wait_for_transaction_with_action(
        &self,
        txhash: impl Into<String>,
        action: Option<Action>,
    ) -> Result<(TxBody, AuthInfo, TxResponse), crate::Error> {
        const DELAY_SECONDS: u64 = 2;
        let txhash = txhash.into();
        for attempt in 1..=self.pool.builder.transaction_attempts() {
            let txres = self
                .perform_query(
                    GetTxRequest {
                        hash: txhash.clone(),
                    },
                    action
                        .clone()
                        .unwrap_or_else(|| Action::WaitForTransaction(txhash.clone())),
                )
                .run()
                .await;
            match txres {
                Ok(txres) => {
                    let txres = txres.into_inner();
                    return Self::txres_to_tuple(
                        txres,
                        action
                            .clone()
                            .unwrap_or_else(|| Action::WaitForTransaction(txhash.clone())),
                    );
                }
                Err(QueryError {
                    // Some nodes will hang on these queries, so treat
                    // QueryTimeout the same as NotFound.
                    query: QueryErrorDetails::NotFound(_) | QueryErrorDetails::QueryTimeout(_),
                    ..
                }) => {
                    tracing::debug!(
                        "Transaction {txhash} not ready, attempt #{attempt}/{}",
                        self.pool.builder.transaction_attempts()
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(DELAY_SECONDS)).await;
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
        Err(match action {
            None => crate::Error::WaitForTransactionTimedOut { txhash },
            Some(action) => crate::Error::WaitForTransactionTimedOutWhile {
                txhash,
                action: action.into(),
            },
        })
    }

    /// Get a list of txhashes for transactions send by the given address.
    pub async fn list_transactions_for(
        &self,
        address: Address,
        limit: Option<u64>,
        page: Option<u64>,
    ) -> Result<Vec<String>, QueryError> {
        // The pagination field within this struct is
        // deprecated. https://docs.rs/cosmos-sdk-proto/0.21.1/cosmos_sdk_proto/cosmos/tx/v1beta1/struct.GetTxsEventRequest.html#structfield.pagination
        #[allow(deprecated)]
        let req = GetTxsEventRequest {
            events: vec![],
            pagination: None,
            order_by: OrderBy::Asc as i32,
            page: page.unwrap_or(1),
            limit: limit.unwrap_or(10),
            query: format!("message.sender='{address}'"),
        };
        self.perform_query(req, Action::ListTransactionsFor(address))
            .run()
            .await
            .map(|x| {
                x.into_inner()
                    .tx_responses
                    .into_iter()
                    .map(|x| x.txhash)
                    .collect()
            })
    }

    /// Get transactions meeting the given query.
    pub async fn query_transactions(
        &self,
        query: String,
        limit: Option<u64>,
        page: Option<u64>,
        order_by: OrderBy,
    ) -> Result<Vec<String>, QueryError> {
        // The pagination field within this struct is
        // deprecated. https://docs.rs/cosmos-sdk-proto/0.21.1/cosmos_sdk_proto/cosmos/tx/v1beta1/struct.GetTxsEventRequest.html#structfield.pagination
        #[allow(deprecated)]
        let req = GetTxsEventRequest {
            events: vec![],
            pagination: None,
            order_by: order_by as i32,
            page: page.unwrap_or(1),
            limit: limit.unwrap_or(10),
            query: query.clone(),
        };
        self.perform_query(
            req,
            Action::QueryTransactions {
                query,
                limit,
                page,
                order_by,
            },
        )
        .run()
        .await
        .map(|x| {
            // FIXME turn into a more full response
            x.into_inner()
                .tx_responses
                .into_iter()
                .map(|x| x.txhash)
                .collect()
        })
    }

    /// attempt_number starts at 0
    async fn gas_to_coins(&self, gas: u64, attempt_number: u64) -> u64 {
        let CurrentGasPrice { low, high, base: _ } = self.current_gas_price().await;
        let attempts = self.pool.builder.gas_price_retry_attempts();

        let gas_price = if attempt_number >= attempts {
            high
        } else {
            assert!(attempts > 0);
            let step = (high - low) / attempts as f64;
            low + step * attempt_number as f64
        };

        (gas as f64 * gas_price).ceil() as u64
    }

    /// Get information on the given block height.
    pub async fn get_block_info(&self, height: i64) -> Result<BlockInfo, crate::Error> {
        let action = Action::GetBlock(height);
        let res = self
            .perform_query(GetBlockByHeightRequest { height }, action.clone())
            .run()
            .await?
            .into_inner();
        BlockInfo::new(action, res.block_id, res.sdk_block, res.block, Some(height))
    }

    /// Same as [Self::get_transaction_with_fallbacks] but for [Self::get_block_info]
    pub async fn get_block_info_with_fallbacks(
        &self,
        height: i64,
    ) -> Result<BlockInfo, crate::Error> {
        let action = Action::GetBlock(height);
        let res = self
            .perform_query(GetBlockByHeightRequest { height }, action.clone())
            .run()
            .await
            .map(|x| x.into_inner());
        match res {
            Ok(res) => BlockInfo::new(action, res.block_id, res.sdk_block, res.block, Some(height)),
            Err(e) => {
                for node in self.pool.node_chooser.all_nodes() {
                    let _permit = self.pool.get_node_permit().await;
                    if let Ok(res) = self
                        .perform_query_inner(GetBlockByHeightRequest { height }, node)
                        .await
                    {
                        let res = res.into_inner();
                        return BlockInfo::new(
                            action,
                            res.block_id,
                            res.sdk_block,
                            res.block,
                            Some(height),
                        );
                    }
                }
                Err(e.into())
            }
        }
    }

    /// Get information on the earliest block available from this node
    pub async fn get_earliest_block_info(&self) -> Result<BlockInfo, crate::Error> {
        match self.get_block_info(1).await {
            Err(crate::Error::Query(QueryError {
                query:
                    QueryErrorDetails::HeightNotAvailable {
                        lowest_height: Some(lowest_height),
                        ..
                    },
                ..
            })) => self.get_block_info(lowest_height).await,
            x => x,
        }
    }

    /// Get the latest block available
    pub async fn get_latest_block_info(&self) -> Result<BlockInfo, crate::Error> {
        let action = Action::GetLatestBlock;
        let res = self
            .perform_query(GetLatestBlockRequest {}, action.clone())
            .run()
            .await?
            .into_inner();
        BlockInfo::new(action, res.block_id, res.sdk_block, res.block, None)
    }

    /// Get the most recently seen block height.
    ///
    /// If no queries have been made, this will return 0.
    pub fn get_last_seen_block(&self) -> i64 {
        self.tracking.block_height.lock().height
    }

    /// Do we think that the chain is currently paused?
    ///
    /// At the moment, this only occurs on Osmosis Mainnet during the epoch.
    pub fn is_chain_paused(&self) -> bool {
        self.chain_paused_status.is_paused()
    }

    /// Get the base gas price.
    ///
    /// On Osmosis mainnet, this will be the base gas fee reported by the chain.
    /// On all other chains, it will be the low price value.
    pub async fn get_base_gas_price(&self) -> f64 {
        self.current_gas_price().await.base
    }

    async fn current_gas_price(&self) -> CurrentGasPrice {
        match &self.get_cosmos_builder().gas_price_method {
            Some(method) => method.current(self).await,
            None => DEFAULT_GAS_PRICE,
        }
    }

    /// Get a node health report
    pub fn node_health_report(&self) -> NodeHealthReport {
        self.pool.node_chooser.health_report()
    }

    /// Get the first block with a timestamp greater than or equal to the given timestamp.
    ///
    /// Takes an optional earliest block to start checking from.
    pub async fn first_block_after(
        &self,
        timestamp: DateTime<Utc>,
        earliest: Option<i64>,
    ) -> Result<i64, FirstBlockAfterError> {
        let earliest = match earliest {
            None => self.get_earliest_block_info().await?,
            Some(height) => self.get_block_info(height).await?,
        };
        let latest = self.get_latest_block_info().await?;
        if earliest.timestamp > timestamp {
            return Err(FirstBlockAfterError::NoBlocksExistBefore {
                timestamp,
                earliest_height: earliest.height,
                earliest_timestamp: earliest.timestamp,
            });
        }
        if latest.timestamp < timestamp {
            return Err(FirstBlockAfterError::NoBlocksExistAfter {
                timestamp,
                latest_height: latest.height,
                latest_timestamp: latest.timestamp,
            });
        }
        let mut low = earliest.height;
        let mut high = latest.height;
        tracing::debug!("Earliest height {low} at {}", earliest.timestamp);
        tracing::debug!("Latest height {high} at {}", latest.timestamp);
        loop {
            if low == high || low + 1 == high {
                break Ok(high);
            }
            assert!(low < high);
            let mid = (high + low) / 2;
            let info = self.get_block_info(mid).await?;
            tracing::debug!(
                "Block #{} occurred at timestamp {}",
                info.height,
                info.timestamp
            );
            if info.timestamp < timestamp {
                low = mid;
            } else {
                high = mid;
            }
        }
    }

    /// Helper function: parse out a raw transaction from encoded bytes.
    ///
    /// This is useful in parsing a transaction created from a frontend.
    pub fn parse_tx_from_bytes<BodyBytes, AuthInfoBytes, Signatures, Signature>(
        body_bytes: BodyBytes,
        auth_info_bytes: AuthInfoBytes,
        signatures: Signatures,
    ) -> Result<Tx, prost::DecodeError>
    where
        BodyBytes: AsRef<[u8]>,
        AuthInfoBytes: AsRef<[u8]>,
        Signatures: IntoIterator<Item = Signature>,
        Signature: AsRef<[u8]>,
    {
        Ok(Tx {
            body: Some(TxBody::decode(body_bytes.as_ref())?),
            auth_info: Some(AuthInfo::decode(auth_info_bytes.as_ref())?),
            signatures: signatures
                .into_iter()
                .map(|signature| signature.as_ref().to_owned())
                .collect(),
        })
    }

    /// Attempt to broadcast a fully formed [Tx]
    pub async fn broadcast_tx_raw(&self, tx: Tx) -> Result<BroadcastTxResponse, QueryError> {
        let PerformQueryWrapper { grpc_url: _, tonic } = self
            .perform_query(
                BroadcastTxRequest {
                    tx_bytes: tx.encode_to_vec(),
                    mode: BroadcastMode::Sync as i32,
                },
                Action::BroadcastRaw,
            )
            .all_nodes()
            .run()
            .await?;
        Ok(tonic.into_inner())
    }
}

/// Information on a block.
#[derive(Debug)]
pub struct BlockInfo {
    /// Block height
    pub height: i64,
    /// Hash of the block
    pub block_hash: String,
    /// Timestamp of the block
    pub timestamp: DateTime<Utc>,
    /// Transaction hashes contained in this block
    pub txhashes: Vec<String>,
    /// Chain ID this block is associated with
    pub chain_id: String,
}

impl BlockInfo {
    fn new(
        action: Action,
        block_id: Option<tendermint_proto::types::BlockId>,
        sdk_block: Option<cosmos_sdk_proto::cosmos::base::tendermint::v1beta1::Block>,
        block: Option<tendermint_proto::types::Block>,
        height: Option<i64>,
    ) -> Result<BlockInfo, crate::Error> {
        (|| {
            let block_id = block_id.ok_or("get_block_info: block_id is None".to_owned())?;
            let (timestamp, header_height, chain_id, data) = match (sdk_block, block) {
                (Some(sdk_block), _) => {
                    let header = sdk_block
                        .header
                        .ok_or("get_block_info: header is None".to_owned())?;
                    let time = header
                        .time
                        .ok_or("get_block_info: time is None".to_owned())?;
                    let timestamp =
                        Utc.timestamp_nanos(time.seconds * 1_000_000_000 + i64::from(time.nanos));
                    (timestamp, header.height, header.chain_id, sdk_block.data)
                }
                (None, Some(block)) => {
                    let header = block
                        .header
                        .ok_or("get_block_info: header is None".to_owned())?;
                    let time = header
                        .time
                        .ok_or("get_block_info: time is None".to_owned())?;
                    let timestamp =
                        Utc.timestamp_nanos(time.seconds * 1_000_000_000 + i64::from(time.nanos));
                    (timestamp, header.height, header.chain_id, block.data)
                }
                (None, None) => return Err("get_block_info: block is None".to_owned()),
            };
            let data = data.ok_or("get_block_info: data is None".to_owned())?;
            if let Some(height) = height {
                if height != header_height {
                    return Err(format!(
                        "Mismatched height from blockchain. Got {header_height}, expected {height}"
                    ));
                }
            }
            let mut txhashes = vec![];
            for tx in data.txs {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(tx);
                let digest = hasher.finalize();
                txhashes.push(hex::encode_upper(digest));
            }
            Ok(BlockInfo {
                height: header_height,
                block_hash: hex::encode_upper(block_id.hash),
                timestamp,
                txhashes,
                chain_id,
            })
        })()
        .map_err(|message| crate::Error::InvalidChainResponse {
            message,
            action: action.into(),
        })
    }
}

impl TxBuilder {
    /// Simulate the transaction with the given signer or signers.
    ///
    /// Note that for simulation purposes you do not need to provide valid
    /// signatures, so only the signer addresses are needed.
    pub async fn simulate(
        &self,
        cosmos: &Cosmos,
        wallets: &[Address],
    ) -> Result<FullSimulateResponse, crate::Error> {
        let mut sequences = vec![];
        for wallet in wallets {
            let base_account = cosmos
                .get_and_update_simulation_sequence(wallet.get_address())
                .await;
            let sequence = match base_account {
                Ok(account) => account.sequence,
                Err(err) => {
                    if err.to_string().contains("not found") {
                        tracing::warn!(
                            "Simulating with a non-existent wallet. Setting sequence number to 0"
                        );
                        0
                    } else {
                        return Err(err);
                    }
                }
            };
            sequences.push(sequence);
        }

        let result = self.simulate_inner(cosmos, &sequences).await;
        if let Err(err) = &result {
            if wallets.len() == 1 {
                let err = err.get_sequence_mismatch_status();
                if let Some(status) = err {
                    let sequence = cosmos.get_expected_sequence(status.message());
                    match sequence {
                        Some(new_sequence_no) => {
                            let result = self.simulate_inner(cosmos, &[new_sequence_no]).await;
                            if result.is_ok() {
                                tracing::info!("Retry of broadcast simulation failure succeeded with new sequence number of {new_sequence_no}");
                            } else {
                                tracing::warn!("Retry of broadcast simulation failed for sequence number {new_sequence_no}");
                            };
                            return result;
                        }
                        None => return result,
                    }
                }
            }
        }
        result
    }

    /// Sign transaction, broadcast, wait for it to complete, confirm that it was successful
    /// the gas amount is determined automatically by running a simulation first and padding by a multiplier
    /// the multiplier can by adjusted by calling [CosmosBuilder::set_gas_estimate_multiplier]
    pub async fn sign_and_broadcast(
        &self,
        cosmos: &Cosmos,
        wallet: &Wallet,
    ) -> Result<TxResponse, crate::Error> {
        self.sign_and_broadcast_cosmos_tx(cosmos, wallet)
            .await
            .map(|cosmos| cosmos.response)
    }

    /// Same as sign_and_broadcast but returns [CosmosTxResponse]
    pub async fn sign_and_broadcast_cosmos_tx(
        &self,
        cosmos: &Cosmos,
        wallet: &Wallet,
    ) -> Result<CosmosTxResponse, crate::Error> {
        let mut attempts = 0;
        loop {
            let simres = self.simulate(cosmos, &[wallet.get_address()]).await?;
            let res = self
                .inner_sign_and_broadcast_cosmos(
                    cosmos,
                    wallet,
                    simres.body,
                    // Gas estimation is not perfect, so we need to adjust it by a multiplier to account for drift
                    // Since we're already estimating and padding, the loss of precision from f64 to u64 is negligible
                    (simres.gas_used as f64 * cosmos.gas_multiplier.get_current()) as u64,
                )
                .await;
            let did_update = cosmos.gas_multiplier.update(&res);
            if !did_update {
                break res;
            }
            let e = match res {
                Ok(x) => break Ok(x),
                Err(e) => e,
            };

            // We know we updated, and that we have an error. That error must
            // be an "out of gas" otherwise we wouldn't have updated the gas multiplier. And we
            // also know that we're using dynamic gas. Now we need to check if we should retry.

            attempts += 1;
            let allowed = cosmos.get_cosmos_builder().get_dynamic_gas_retries();
            if attempts >= cosmos.get_cosmos_builder().get_dynamic_gas_retries() {
                break Err(e);
            }
            tracing::warn!(
                "Out of gas while executing transaction, retrying ({attempts}/{allowed}): {e}"
            );
        }
    }

    /// Sign transaction, broadcast, wait for it to complete, confirm that it was successful
    /// unlike sign_and_broadcast(), the gas amount is explicit here and therefore no simulation is run
    pub async fn sign_and_broadcast_with_gas(
        &self,
        cosmos: &Cosmos,
        wallet: &Wallet,
        gas_to_request: u64,
    ) -> Result<TxResponse, crate::Error> {
        self.inner_sign_and_broadcast_cosmos(cosmos, wallet, self.make_tx_body(), gas_to_request)
            .await
            .map(|cosmos| cosmos.response)
    }

    /// Same as [sign_and_broadcast_with_gas] but returns [CosmosTxResponse]
    pub async fn sign_and_broadcast_with_cosmos_gas(
        &self,
        cosmos: &Cosmos,
        wallet: &Wallet,
        gas_to_request: u64,
    ) -> Result<CosmosTxResponse, crate::Error> {
        let base_account = cosmos
            .get_and_update_broadcast_sequence(wallet.get_address())
            .await?;
        self.sign_and_broadcast_with_inner(
            cosmos,
            wallet,
            &base_account,
            base_account.sequence,
            self.make_tx_body(),
            gas_to_request,
        )
        .await
    }

    async fn inner_sign_and_broadcast_cosmos(
        &self,
        cosmos: &Cosmos,
        wallet: &Wallet,
        body: TxBody,
        gas_to_request: u64,
    ) -> Result<CosmosTxResponse, crate::Error> {
        let base_account = cosmos
            .get_and_update_broadcast_sequence(wallet.get_address())
            .await?;
        self.sign_and_broadcast_with_cosmos_tx(
            cosmos,
            wallet,
            &base_account,
            base_account.sequence,
            body.clone(),
            gas_to_request,
        )
        .await
    }

    fn make_signer_info(&self, sequence: u64, wallet: Option<&Wallet>) -> SignerInfo {
        SignerInfo {
            public_key: match wallet {
                // No wallet/base account. We're simulating. Fill in a dummy value.
                None => Some(cosmos_sdk_proto::Any {
                    type_url: "/cosmos.crypto.secp256k1.PubKey".to_owned(),
                    value: cosmos_sdk_proto::tendermint::crypto::PublicKey {
                        sum: Some(
                            cosmos_sdk_proto::tendermint::crypto::public_key::Sum::Ed25519(vec![]),
                        ),
                    }
                    .encode_to_vec(),
                }),
                Some(wallet) => {
                    match wallet.public_key {
                        // Use the Cosmos method of public key
                        WalletPublicKey::Cosmos(public_key) => Some(cosmos_sdk_proto::Any {
                            type_url: "/cosmos.crypto.secp256k1.PubKey".to_owned(),
                            value: cosmos_sdk_proto::tendermint::crypto::PublicKey {
                                sum: Some(
                                    cosmos_sdk_proto::tendermint::crypto::public_key::Sum::Ed25519(
                                        public_key.to_vec(),
                                    ),
                                ),
                            }
                            .encode_to_vec(),
                        }),
                        // Use the Injective method of public key
                        WalletPublicKey::Ethereum(public_key) => Some(cosmos_sdk_proto::Any {
                            type_url: "/injective.crypto.v1beta1.ethsecp256k1.PubKey".to_owned(),
                            value: cosmos_sdk_proto::tendermint::crypto::PublicKey {
                                sum: Some(
                                    cosmos_sdk_proto::tendermint::crypto::public_key::Sum::Ed25519(
                                        public_key.to_vec(),
                                    ),
                                ),
                            }
                            .encode_to_vec(),
                        }),
                    }
                }
            },
            mode_info: Some(ModeInfo {
                sum: Some(
                    cosmos_sdk_proto::cosmos::tx::v1beta1::mode_info::Sum::Single(
                        cosmos_sdk_proto::cosmos::tx::v1beta1::mode_info::Single { mode: 1 },
                    ),
                ),
            }),
            sequence,
        }
    }

    /// Make a [TxBody] for this builder
    fn make_tx_body(&self) -> TxBody {
        TxBody {
            messages: self.messages.iter().map(|msg| msg.get_protobuf()).collect(),
            memo: self.memo.as_deref().unwrap_or_default().to_owned(),
            timeout_height: 0,
            extension_options: vec![],
            non_critical_extension_options: vec![],
        }
    }

    /// Simulate to calculate the gas costs
    async fn simulate_inner(
        &self,
        cosmos: &Cosmos,
        sequences: &[u64],
    ) -> Result<FullSimulateResponse, crate::Error> {
        let body = self.make_tx_body();
        let gas_coin = cosmos.pool.builder.gas_coin();

        // First simulate the request with no signature and fake gas
        #[allow(deprecated)]
        let simulate_tx = Tx {
            auth_info: Some(AuthInfo {
                fee: Some(Fee {
                    amount: if cosmos.pool.builder.get_simulate_with_gas_coin() {
                        vec![Coin {
                            denom: gas_coin.to_owned(),
                            amount: "1".to_owned(),
                        }]
                    } else {
                        vec![]
                    },
                    gas_limit: 0,
                    payer: "".to_owned(),
                    granter: "".to_owned(),
                }),
                signer_infos: sequences
                    .iter()
                    .map(|sequence| self.make_signer_info(*sequence, None))
                    .collect(),
                tip: None,
            }),
            signatures: sequences.iter().map(|_| vec![]).collect(),
            body: Some(body.clone()),
        };

        #[allow(deprecated)]
        let simulate_req = SimulateRequest {
            tx: None,
            tx_bytes: simulate_tx.encode_to_vec(),
        };

        let action = Action::Simulate(self.clone());
        let simres = cosmos
            .perform_query(simulate_req, action.clone())
            .run()
            .await?
            .into_inner();

        let gas_used = simres
            .gas_info
            .as_ref()
            .ok_or_else(|| crate::Error::InvalidChainResponse {
                message: "Missing gas_info in SimulateResponse".to_owned(),
                action: action.into(),
            })?
            .gas_used;

        Ok(FullSimulateResponse {
            body,
            simres,
            gas_used,
        })
    }

    async fn sign_and_broadcast_with_cosmos_tx(
        &self,
        cosmos: &Cosmos,
        wallet: &Wallet,
        base_account: &BaseAccount,
        sequence: u64,
        body: TxBody,
        gas_to_request: u64,
    ) -> Result<CosmosTxResponse, crate::Error> {
        self.sign_and_broadcast_with_inner(
            cosmos,
            wallet,
            base_account,
            sequence,
            body,
            gas_to_request,
        )
        .await
    }

    async fn sign_and_broadcast_with_inner(
        &self,
        cosmos: &Cosmos,
        wallet: &Wallet,
        base_account: &BaseAccount,
        sequence: u64,
        body: TxBody,
        gas_to_request: u64,
    ) -> Result<CosmosTxResponse, crate::Error> {
        // enum AttemptError {
        //     Inner(Infallible),
        //     InsufficientGas(Infallible),
        // }
        // impl From<anyhow::Error> for AttemptError {
        //     fn from(e: anyhow::Error) -> Self {
        //         AttemptError::Inner(e)
        //     }
        // }
        let body_ref = &body;
        let retry_with_price = |amount| async move {
            let amount = Coin {
                denom: cosmos.pool.builder.gas_coin().to_owned(),
                amount,
            };
            #[allow(deprecated)]
            let auth_info = AuthInfo {
                signer_infos: vec![self.make_signer_info(sequence, Some(wallet))],
                fee: Some(Fee {
                    amount: vec![amount.clone()],
                    gas_limit: gas_to_request,
                    payer: "".to_owned(),
                    granter: "".to_owned(),
                }),
                tip: None,
            };

            let sign_doc = SignDoc {
                body_bytes: body_ref.encode_to_vec(),
                auth_info_bytes: auth_info.encode_to_vec(),
                chain_id: cosmos.pool.builder.chain_id().to_owned(),
                account_number: base_account.account_number,
            };
            let sign_doc_bytes = sign_doc.encode_to_vec();
            let signature = wallet.sign_bytes(&sign_doc_bytes);

            let tx = Tx {
                body: Some(body_ref.clone()),
                auth_info: Some(auth_info),
                signatures: vec![signature.serialize_compact().to_vec()],
            };

            let mk_action = move || Action::Broadcast {
                txbuilder: self.clone(),
                gas_wanted: gas_to_request,
                fee: amount.clone(),
            };

            let (grpc_url, res) = cosmos
                .perform_query(
                    BroadcastTxRequest {
                        tx_bytes: tx.encode_to_vec(),
                        mode: BroadcastMode::Sync as i32,
                    },
                    mk_action(),
                )
                .all_nodes()
                .run_broadcast(self.skip_code_check)
                .await?;

            let action = Action::WaitForBroadcast {
                txbuilder: self.clone(),
                txhash: res.txhash.clone(),
            };

            let (_, _, res) = cosmos
                .wait_for_transaction_with_action(res.txhash, Some(action.clone()))
                .await?;
            if !self.skip_code_check && res.code != 0 {
                return Err(crate::Error::TransactionFailed {
                    code: CosmosSdkError::from_code(res.code, &res.codespace),
                    txhash: res.txhash.clone(),
                    raw_log: res.raw_log,
                    action: action.into(),
                    grpc_url,
                    stage: crate::error::TransactionStage::Wait,
                });
            };

            tracing::debug!("TxResponse: {res:?}");
            cosmos
                .update_broadcast_sequence(wallet.get_address(), &tx, &res.txhash)
                .await?;

            Ok(CosmosTxResponse { response: res, tx })
        };

        let attempts = cosmos.get_cosmos_builder().gas_price_retry_attempts();
        for attempt_number in 0..attempts {
            let amount = cosmos
                .gas_to_coins(gas_to_request, attempt_number)
                .await
                .to_string();
            match retry_with_price(amount).await {
                Err(crate::Error::TransactionFailed {
                    code: CosmosSdkError::InsufficientFee,
                    txhash,
                    raw_log,
                    action: _,
                    grpc_url: _,
                    stage: _,
                }) => {
                    tracing::debug!(
                        "Insufficient gas in attempt #{}, retrying {txhash}. Raw log: {raw_log}",
                        attempt_number + 1
                    );
                }
                res => return res,
            }
        }

        let amount = cosmos
            .gas_to_coins(gas_to_request, attempts)
            .await
            .to_string();
        retry_with_price(amount).await
    }

    /// Does this transaction have any messages already?
    pub fn has_messages(&self) -> bool {
        !self.messages.is_empty()
    }
}

/// Trait for any types that contain a [Cosmos] connection.
pub trait HasCosmos: HasAddressHrp {
    /// Get the underlying connection
    fn get_cosmos(&self) -> &Cosmos;
}

impl HasCosmos for Cosmos {
    fn get_cosmos(&self) -> &Cosmos {
        self
    }
}

impl<T: HasCosmos> HasCosmos for &T {
    fn get_cosmos(&self) -> &Cosmos {
        HasCosmos::get_cosmos(*self)
    }
}

/// Returned the expected account sequence mismatch based on an error message, if present.
///
/// Always returns [None] if autofix_sequence_mismatch is disabled (the default).
impl Cosmos {
    fn get_expected_sequence(&self, message: &str) -> Option<u64> {
        let cosmos_builder = self.get_cosmos_builder();
        match cosmos_builder.autofix_simulate_sequence_mismatch {
            Some(true) => get_expected_sequence_inner(message),
            Some(false) => None,
            None => None,
        }
    }
}

fn get_expected_sequence_inner(message: &str) -> Option<u64> {
    for line in message.lines() {
        if let Some(x) = get_expected_sequence_single(line) {
            return Some(x);
        }
    }
    None
}

fn get_expected_sequence_single(message: &str) -> Option<u64> {
    let s = message.strip_prefix("account sequence mismatch, expected ")?;
    let comma = s.find(',')?;
    s[..comma].parse().ok()
}

#[cfg(test)]
mod tests {
    use crate::CosmosNetwork;

    use super::*;

    #[test]
    fn gas_estimate_multiplier() {
        let mut cosmos = CosmosNetwork::OsmosisTestnet.builder_local();

        // the same as sign_and_broadcast()
        let multiply_estimated_gas = |cosmos: &CosmosBuilder, gas_used: u64| -> u64 {
            (gas_used as f64 * cosmos.build_gas_multiplier().get_current()) as u64
        };

        assert_eq!(multiply_estimated_gas(&cosmos, 1234), 1604);
        cosmos.set_gas_estimate_multiplier(4.2);
        assert_eq!(multiply_estimated_gas(&cosmos, 1234), 5182);
    }

    #[tokio::test]
    async fn lazy_load() {
        let mut builder = CosmosNetwork::OsmosisTestnet.builder().await.unwrap();
        builder.set_query_retries(Some(0));
        // something that clearly won't work
        builder.set_grpc_url("https://0.0.0.0:0".to_owned());

        let cosmos = builder.build().unwrap();
        cosmos.get_latest_block_info().await.unwrap_err();
    }

    #[tokio::test]
    async fn fallback() {
        let mut builder = CosmosNetwork::OsmosisTestnet.builder().await.unwrap();
        builder.add_grpc_fallback_url(builder.grpc_url().to_owned());
        builder.set_grpc_url("http://0.0.0.0:0");
        let cosmos = builder.build().unwrap();
        cosmos.get_latest_block_info().await.unwrap();
    }

    #[tokio::test]
    async fn ignore_broken_fallback() {
        let mut builder = CosmosNetwork::OsmosisTestnet.builder().await.unwrap();
        builder.add_grpc_fallback_url("http://0.0.0.0:0");
        let cosmos = builder.build().unwrap();
        cosmos.get_latest_block_info().await.unwrap();
    }

    #[test]
    fn get_expected_sequence_good() {
        assert_eq!(
            get_expected_sequence_inner("account sequence mismatch, expected 5, got 0"),
            Some(5)
        );
        assert_eq!(
            get_expected_sequence_inner("account sequence mismatch, expected 2, got 7"),
            Some(2)
        );
        assert_eq!(
            get_expected_sequence_inner("account sequence mismatch, expected 20000001, got 7"),
            Some(20000001)
        );
    }

    #[test]
    fn get_expected_sequence_extra_prelude() {
        assert_eq!(
            get_expected_sequence_inner(
                "blah blah blah\n\naccount sequence mismatch, expected 5, got 0"
            ),
            Some(5)
        );
        assert_eq!(
            get_expected_sequence_inner(
                "foajodifjaolkdfjas aiodjfaof\n\n\naccount sequence mismatch, expected 2, got 7"
            ),
            Some(2)
        );
        assert_eq!(
            get_expected_sequence_inner(
                "iiiiiiiiiiiiii\n\naccount sequence mismatch, expected 20000001, got 7"
            ),
            Some(20000001)
        );
    }

    #[test]
    fn get_expected_sequence_bad() {
        assert_eq!(
            get_expected_sequence_inner("Totally different error message"),
            None
        );
        assert_eq!(
            get_expected_sequence_inner("account sequence mismatch, expected XXXXX, got 7"),
            None
        );
    }
}

#[derive(Debug)]
pub struct FullSimulateResponse {
    pub body: TxBody,
    pub simres: SimulateResponse,
    pub gas_used: u64,
}
