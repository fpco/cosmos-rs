mod node;
mod node_chooser;
mod pool;
mod query;

use std::{
    str::FromStr,
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
            AuthInfo, BroadcastMode, BroadcastTxRequest, Fee, GetTxRequest, GetTxResponse,
            GetTxsEventRequest, ModeInfo, OrderBy, SignDoc, SignerInfo, SimulateRequest,
            SimulateResponse, Tx, TxBody,
        },
    },
    cosmwasm::wasm::v1::QueryCodeRequest,
    traits::Message,
};
use parking_lot::Mutex;
use tokio::time::Instant;
use tonic::{service::Interceptor, Status};

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
    block_height_tracking: Arc<Mutex<BlockHeightTracking>>,
    pub(crate) chain_paused_status: ChainPausedStatus,
    gas_multiplier: GasMultiplier,
    /// Maximum gas price
    pub(crate) max_price: f64,
}

pub(crate) struct WeakCosmos {
    pool: Pool,
    height: Option<u64>,
    block_height_tracking: Weak<Mutex<BlockHeightTracking>>,
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
            block_height_tracking,
            chain_paused_status,
            gas_multiplier,
            max_price,
        }: &Cosmos,
    ) -> Self {
        WeakCosmos {
            pool: pool.clone(),
            height: *height,
            block_height_tracking: Arc::downgrade(block_height_tracking),
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
            block_height_tracking,
            chain_paused_status,
            gas_multiplier,
            max_price,
        } = self;
        block_height_tracking
            .upgrade()
            .map(|block_height_tracking| Cosmos {
                pool: pool.clone(),
                height: *height,
                block_height_tracking,
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
        let mut guard = self.pool.get().await?;
        let cosmos = guard.get_inner_mut();
        let sequence = {
            let guard = cosmos.simulate_sequences().read();
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
                        let mut seq_info = cosmos.simulate_sequences().write();
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
        let mut seq_info = cosmos.simulate_sequences().write();
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
        let mut guard = self.pool.get().await?;
        let cosmos = guard.get_inner_mut();
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
                    let mut sequences = cosmos.broadcast_sequences().write();
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
        let mut guard = self.pool.get().await?;
        let cosmos = guard.get_inner_mut();
        let sequence = {
            let guard = cosmos.broadcast_sequences().read();
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
                        let mut seq_info = cosmos.broadcast_sequences().write();
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
        let mut seq_info = cosmos.broadcast_sequences().write();
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

    pub(crate) async fn perform_query<Request: GrpcRequest>(
        &self,
        req: Request,
        action: Action,
        should_retry: bool,
    ) -> Result<PerformQueryWrapper<Request::Response>, QueryError> {
        let mut attempt = 0;
        loop {
            let (err, can_retry, grpc_url) = match self.pool.get().await {
                Err(err) => (
                    QueryErrorDetails::ConnectionError(err),
                    true,
                    self.get_cosmos_builder().grpc_url_arc().clone(),
                ),
                Ok(mut guard) => {
                    let cosmos_inner = guard.get_inner_mut();
                    if self.pool.builder.get_log_requests() {
                        tracing::info!("{action}");
                    }
                    match self.perform_query_inner(req.clone(), cosmos_inner).await {
                        Ok(x) => {
                            cosmos_inner.log_query_result(QueryResult::Success);
                            break Ok(PerformQueryWrapper {
                                grpc_url: cosmos_inner.grpc_url().clone(),
                                tonic: x,
                            });
                        }
                        Err((err, can_retry)) => {
                            cosmos_inner.log_query_result(if can_retry {
                                QueryResult::NetworkError {
                                    err: err.clone(),
                                    action: action.clone(),
                                }
                            } else {
                                QueryResult::OtherError
                            });
                            (err, can_retry, cosmos_inner.grpc_url().clone())
                        }
                    }
                }
            };
            if attempt >= self.pool.builder.query_retries() || !should_retry || !can_retry {
                break Err(QueryError {
                    action,
                    builder: self.pool.builder.clone(),
                    height: self.height,
                    query: err,
                    grpc_url,
                    node_health: self.pool.node_chooser.health_report(),
                });
            } else {
                attempt += 1;
                tracing::debug!(
                    "Error performing a query, retrying. Attempt {attempt} of {}. {err:?}",
                    self.pool.builder.query_retries()
                );
            }
        }
    }

    /// Error return: the details itself, and whether a retry can be attempted.
    async fn perform_query_inner<Request: GrpcRequest>(
        &self,
        req: Request,
        cosmos_inner: &mut Node,
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
                            .set_broken(|grpc_url| ConnectionError::QueryFailed { grpc_url });
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
                                cosmos_inner.set_broken(|grpc_url| {
                                    ConnectionError::SanityCheckFailed {
                                        grpc_url,
                                        source: status,
                                    }
                                });
                                true
                            }
                        }
                    }
                };

                Err((err, can_retry))
            }
            Err(_) => {
                cosmos_inner.set_broken(|grpc_url| ConnectionError::TimeoutQuery { grpc_url });
                Err((QueryErrorDetails::QueryTimeout(duration), true))
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
            Ok(new_height) => new_height,
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

        let mut guard = self.block_height_tracking.lock();

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
        if old_height - new_height > self.get_cosmos_builder().block_lag_allowed().into() {
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
pub struct CosmosInterceptor(Option<Arc<String>>);

impl Interceptor for CosmosInterceptor {
    fn call(&mut self, mut request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        let req = request.metadata_mut();
        if let Some(value) = &self.0 {
            let value = FromStr::from_str(value);
            if let Ok(header_value) = value {
                req.insert("referer", header_value);
            }
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
    /// Create a new [Cosmos] and perform a sanity check to make sure the connection works.
    pub async fn build(self) -> Result<Cosmos, BuilderError> {
        let cosmos = self.build_lazy()?;

        let resp = cosmos
            .perform_query(GetLatestBlockRequest {}, Action::SanityCheck, false)
            .await
            .map_err(|source| BuilderError::SanityQueryFailed { source })?;

        let actual = resp
            .into_inner()
            .block
            .and_then(|block| block.header)
            .map(|header| header.chain_id);

        let expected = cosmos.get_cosmos_builder().chain_id();
        if actual.as_deref() == Some(expected) {
            Ok(cosmos)
        } else {
            Err(BuilderError::MismatchedChainIds {
                grpc_url: cosmos.get_cosmos_builder().grpc_url().to_owned(),
                expected: expected.to_owned(),
                actual,
            })
        }
    }

    /// Create a new [Cosmos] but do not perform any sanity checks.
    ///
    /// Can fail if parsing the gRPC URLs fails.
    pub fn build_lazy(self) -> Result<Cosmos, BuilderError> {
        let builder = Arc::new(self);
        let chain_paused_status = builder.chain_paused_method.into();
        let gas_multiplier = builder.build_gas_multiplier();
        let max_price = builder.get_init_max_gas_price();
        let cosmos = Cosmos {
            pool: Pool::new(builder)?,
            height: None,
            block_height_tracking: Arc::new(Mutex::new(BlockHeightTracking {
                when: Instant::now(),
                height: 0,
            })),
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
                true,
            )
            .await?
            .into_inner();

        let base_account = if self.get_address_hrp().as_str() == "inj" {
            let eth_account: crate::injective::EthAccount = prost::Message::decode(
                res.account
                    .ok_or_else(|| crate::Error::InvalidChainResponse {
                        message: "no eth account found".to_owned(),
                        action: action.clone(),
                    })?
                    .value
                    .as_ref(),
            )
            .map_err(|source| crate::Error::InvalidChainResponse {
                message: format!("Unable to parse eth_account: {source}"),
                action: action.clone(),
            })?;
            eth_account
                .base_account
                .ok_or_else(|| crate::Error::InvalidChainResponse {
                    message: "no base account found".to_owned(),
                    action: action.clone(),
                })?
        } else {
            prost::Message::decode(
                res.account
                    .ok_or_else(|| crate::Error::InvalidChainResponse {
                        message: "no account found".to_owned(),
                        action: action.clone(),
                    })?
                    .value
                    .as_ref(),
            )
            .map_err(|source| crate::Error::InvalidChainResponse {
                message: format!("Unable to parse account: {source}"),
                action,
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
                    },
                    Action::QueryAllBalances(address),
                    true,
                )
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
            .perform_query(
                QueryCodeRequest { code_id },
                Action::CodeInfo(code_id),
                true,
            )
            .await?;
        Ok(res.into_inner().data)
    }

    fn txres_to_pair(
        txres: GetTxResponse,
        action: Action,
    ) -> Result<(TxBody, TxResponse), crate::Error> {
        let txbody = txres
            .tx
            .ok_or_else(|| crate::Error::InvalidChainResponse {
                message: "Missing tx field".to_owned(),
                action: action.clone(),
            })?
            .body
            .ok_or_else(|| crate::Error::InvalidChainResponse {
                message: "Missing tx.body field".to_owned(),
                action: action.clone(),
            })?;
        let txres = txres
            .tx_response
            .ok_or_else(|| crate::Error::InvalidChainResponse {
                message: "Missing tx_response field".to_owned(),
                action: action.clone(),
            })?;
        Ok((txbody, txres))
    }

    /// Get a transaction, failing immediately if not present
    ///
    /// This will follow normal fallback rules for other queries. You may want
    /// to try out [Self::get_transaction_with_fallbacks].
    pub async fn get_transaction_body(
        &self,
        txhash: impl Into<String>,
    ) -> Result<(TxBody, TxResponse), crate::Error> {
        let txhash = txhash.into();
        let action = Action::GetTransactionBody(txhash.clone());
        let txres = self
            .perform_query(
                GetTxRequest {
                    hash: txhash.clone(),
                },
                action.clone(),
                true,
            )
            .await?
            .into_inner();
        Self::txres_to_pair(txres, action)
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
    ) -> Result<(TxBody, TxResponse), crate::Error> {
        let txhash = txhash.into();
        let action = Action::GetTransactionBody(txhash.clone());
        let res = self
            .perform_query(
                GetTxRequest {
                    hash: txhash.clone(),
                },
                action.clone(),
                true,
            )
            .await;
        match res {
            Ok(txres) => Self::txres_to_pair(txres.into_inner(), action),
            Err(e) => {
                for node in self.pool.node_chooser.all_nodes() {
                    if let Ok(mut node_guard) = self.pool.get_with_node(node).await {
                        if let Ok(txres) = self
                            .perform_query_inner(
                                GetTxRequest {
                                    hash: txhash.clone(),
                                },
                                node_guard.get_inner_mut(),
                            )
                            .await
                        {
                            return Self::txres_to_pair(txres.into_inner(), action);
                        }
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
    ) -> Result<(TxBody, TxResponse), crate::Error> {
        self.wait_for_transaction_with_action(txhash, None).await
    }

    async fn wait_for_transaction_with_action(
        &self,
        txhash: impl Into<String>,
        action: Option<Action>,
    ) -> Result<(TxBody, TxResponse), crate::Error> {
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
                    false,
                )
                .await;
            match txres {
                Ok(txres) => {
                    let txres = txres.into_inner();
                    return Self::txres_to_pair(
                        txres,
                        action
                            .clone()
                            .unwrap_or_else(|| Action::WaitForTransaction(txhash.clone())),
                    );
                }
                Err(QueryError {
                    query: QueryErrorDetails::NotFound(_),
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
            Some(action) => crate::Error::WaitForTransactionTimedOutWhile { txhash, action },
        })
    }

    /// Get a list of txhashes for transactions send by the given address.
    pub async fn list_transactions_for(
        &self,
        address: Address,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<Vec<String>, QueryError> {
        self.perform_query(
            GetTxsEventRequest {
                events: vec![format!("message.sender='{address}'")],
                pagination: Some(PageRequest {
                    key: vec![],
                    offset: offset.unwrap_or_default(),
                    limit: limit.unwrap_or(10),
                    count_total: false,
                    reverse: false,
                }),
                order_by: OrderBy::Asc as i32,
            },
            Action::ListTransactionsFor(address),
            true,
        )
        .await
        .map(|x| {
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
            .perform_query(GetBlockByHeightRequest { height }, action.clone(), true)
            .await?
            .into_inner();
        BlockInfo::new(action, res.block_id, res.block, Some(height))
    }

    /// Same as [Self::get_transaction_with_fallbacks] but for [Self::get_block_info]
    pub async fn get_block_info_with_fallbacks(
        &self,
        height: i64,
    ) -> Result<BlockInfo, crate::Error> {
        let action = Action::GetBlock(height);
        let res = self
            .perform_query(GetBlockByHeightRequest { height }, action.clone(), true)
            .await
            .map(|x| x.into_inner());
        match res {
            Ok(res) => BlockInfo::new(action, res.block_id, res.block, Some(height)),
            Err(e) => {
                for node in self.pool.node_chooser.all_nodes() {
                    if let Ok(mut node_guard) = self.pool.get_with_node(node).await {
                        if let Ok(res) = self
                            .perform_query_inner(
                                GetBlockByHeightRequest { height },
                                node_guard.get_inner_mut(),
                            )
                            .await
                        {
                            let res = res.into_inner();
                            return BlockInfo::new(action, res.block_id, res.block, Some(height));
                        }
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
            .perform_query(GetLatestBlockRequest {}, action.clone(), true)
            .await?
            .into_inner();
        BlockInfo::new(action, res.block_id, res.block, None)
    }

    /// Get the most recently seen block height.
    ///
    /// If no queries have been made, this will return 0.
    pub fn get_last_seen_block(&self) -> i64 {
        self.block_height_tracking.lock().height
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
        block_id: Option<cosmos_sdk_proto::tendermint::types::BlockId>,
        block: Option<cosmos_sdk_proto::tendermint::types::Block>,
        height: Option<i64>,
    ) -> Result<BlockInfo, crate::Error> {
        (|| {
            let block_id = block_id.ok_or("get_block_info: block_id is None".to_owned())?;
            let block = block.ok_or("get_block_info: block is None".to_owned())?;
            let header = block
                .header
                .ok_or("get_block_info: header is None".to_owned())?;
            let time = header
                .time
                .ok_or("get_block_info: time is None".to_owned())?;
            let data = block
                .data
                .ok_or("get_block_info: data is None".to_owned())?;
            if let Some(height) = height {
                if height != header.height {
                    return Err(format!(
                        "Mismatched height from blockchain. Got {}, expected {height}",
                        header.height
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
                height: header.height,
                block_hash: hex::encode_upper(block_id.hash),
                timestamp: Utc
                    .timestamp_nanos(time.seconds * 1_000_000_000 + i64::from(time.nanos)),
                txhashes,
                chain_id: header.chain_id,
            })
        })()
        .map_err(|message| crate::Error::InvalidChainResponse { message, action })
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

        // First simulate the request with no signature and fake gas
        let simulate_tx = Tx {
            auth_info: Some(AuthInfo {
                fee: Some(Fee {
                    amount: vec![],
                    gas_limit: 0,
                    payer: "".to_owned(),
                    granter: "".to_owned(),
                }),
                signer_infos: sequences
                    .iter()
                    .map(|sequence| self.make_signer_info(*sequence, None))
                    .collect(),
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
            .perform_query(simulate_req, action.clone(), true)
            .await?
            .into_inner();

        let gas_used = simres
            .gas_info
            .as_ref()
            .ok_or_else(|| crate::Error::InvalidChainResponse {
                message: "Missing gas_info in SimulateResponse".to_owned(),
                action,
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
            let auth_info = AuthInfo {
                signer_infos: vec![self.make_signer_info(sequence, Some(wallet))],
                fee: Some(Fee {
                    amount: vec![Coin {
                        denom: cosmos.pool.builder.gas_coin().to_owned(),
                        amount,
                    }],
                    gas_limit: gas_to_request,
                    payer: "".to_owned(),
                    granter: "".to_owned(),
                }),
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

            let PerformQueryWrapper { grpc_url, tonic } = cosmos
                .perform_query(
                    BroadcastTxRequest {
                        tx_bytes: tx.encode_to_vec(),
                        mode: BroadcastMode::Sync as i32,
                    },
                    Action::Broadcast(self.clone()),
                    true,
                )
                .await?;
            let res = tonic.into_inner().tx_response.ok_or_else(|| {
                crate::Error::InvalidChainResponse {
                    message: "Missing inner tx_response".to_owned(),
                    action: Action::Broadcast(self.clone()),
                }
            })?;

            if !self.skip_code_check && res.code != 0 {
                return Err(crate::Error::TransactionFailed {
                    code: res.code.into(),
                    raw_log: res.raw_log,
                    action: Action::Broadcast(self.clone()).into(),
                    grpc_url,
                    stage: crate::error::TransactionStage::Broadcast,
                });
            };

            tracing::debug!("Initial BroadcastTxResponse: {res:?}");

            let (_, res) = cosmos
                .wait_for_transaction_with_action(res.txhash, Some(Action::Broadcast(self.clone())))
                .await?;
            if !self.skip_code_check && res.code != 0 {
                return Err(crate::Error::TransactionFailed {
                    code: res.code.into(),
                    raw_log: res.raw_log,
                    action: Action::Broadcast(self.clone()).into(),
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
                    raw_log,
                    action: _,
                    grpc_url: _,
                    stage: _,
                }) => {
                    tracing::debug!(
                        "Insufficient gas in attempt #{}, retrying. Raw log: {raw_log}",
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

        builder.clone().build().await.unwrap_err();
        let cosmos = builder.build_lazy().unwrap();
        cosmos.get_latest_block_info().await.unwrap_err();
    }

    #[tokio::test]
    async fn fallback() {
        let mut builder = CosmosNetwork::OsmosisTestnet.builder().await.unwrap();
        builder.set_allowed_error_count(Some(0));
        builder.add_grpc_fallback_url(builder.grpc_url().to_owned());
        builder.set_grpc_url("http://0.0.0.0:0");
        let cosmos = builder.build_lazy().unwrap();
        cosmos.get_latest_block_info().await.unwrap();
    }

    #[tokio::test]
    async fn ignore_broken_fallback() {
        let mut builder = CosmosNetwork::OsmosisTestnet.builder().await.unwrap();
        builder.set_allowed_error_count(Some(0));
        builder.add_grpc_fallback_url("http://0.0.0.0:0");
        let cosmos = builder.build_lazy().unwrap();
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
