#![allow(missing_docs)]
//! Error types exposed by this package.

use std::{fmt::Display, path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use bip39::Mnemonic;
use bitcoin::bip32::DerivationPath;
use chrono::{DateTime, Utc};
use cosmos_sdk_proto::cosmos::tx::v1beta1::OrderBy;
use http::uri::InvalidUri;

use crate::{Address, AddressHrp, CosmosBuilder, TxBuilder};

/// Errors that can occur with token factory
#[derive(thiserror::Error, Debug, Clone)]
pub enum TokenFactoryError {
    #[error("cosmos-rs does not support tokenfactory for the given chain HRP: {hrp}")]
    Unsupported { hrp: AddressHrp },
}

/// Errors that can occur while working with [crate::Address].
#[derive(thiserror::Error, Debug, Clone)]
pub enum AddressError {
    #[error("Invalid bech32 encoding in {address:?}: {source:?}")]
    InvalidBech32 {
        address: String,
        source: bech32::DecodeError,
    },
    #[error("Invalid byte count within {address:?}, expected 20 or 32 bytes, received {actual}")]
    InvalidByteCount { address: String, actual: usize },
    #[error("Invalid HRP provided: {hrp:?}")]
    InvalidHrp { hrp: String },
}

/// Errors that can occur while working with [crate::Wallet].

#[derive(thiserror::Error, Debug, Clone)]
pub enum WalletError {
    #[error("Could not get root private key from mnemonic: {source:?}")]
    CouldNotGetRootPrivateKey { source: bitcoin::bip32::Error },
    #[error("Could not derive private key using derivation path {derivation_path}: {source:?}")]
    CouldNotDerivePrivateKey {
        derivation_path: Arc<DerivationPath>,
        source: bitcoin::bip32::Error,
    },
    #[error("Invalid derivation path {path:?}: {source:?}")]
    InvalidDerivationPath {
        path: String,
        source: <DerivationPath as FromStr>::Err,
    },
    #[error("Invalid seed phrase: {source}")]
    InvalidPhrase { source: <Mnemonic as FromStr>::Err },
}

/// Error while parsing a [crate::ParsedCoin].
#[derive(thiserror::Error, Debug, Clone)]
pub enum ParsedCoinError {
    #[error("Input is empty")]
    EmptyInput,
    #[error("No amount found in {input:?}")]
    NoAmountFound { input: String },
    #[error("No denom found in {input:?}")]
    NoDenomFound { input: String },
    #[error("Invalid denom: {input:?}")]
    InvalidDenom { input: String },
    #[error("Invalid amount: {input:?}: {source:?}")]
    InvalidAmount {
        input: String,
        source: std::num::ParseIntError,
    },
}

/// Errors that can occur while building a connection.
#[derive(thiserror::Error, Debug)]
pub enum BuilderError {
    #[error("Invalid gRPC URL: {grpc_url}: {source:?}")]
    InvalidGrpcUrl {
        grpc_url: Arc<String>,
        source: Arc<tonic::transport::Error>,
    },
    #[error("Invalid Origin URI: {gprc_url}: {source}")]
    InvalidUri {
        gprc_url: Arc<String>,
        source: InvalidUri,
    },
    #[error("Unable to configure TLS for {grpc_url}: {source:?}")]
    TlsConfig {
        grpc_url: Arc<String>,
        source: Arc<tonic::transport::Error>,
    },
    #[error("Error downloading chain information from {url}: {source:?}")]
    DownloadChainInfo { url: String, source: reqwest::Error },
    #[error("Unknown Cosmos network value {network:?}")]
    UnknownCosmosNetwork { network: String },
    #[error("Mismatched chain IDs during sanity check of {grpc_url}. Expected: {expected}. Actual: {actual:?}.")]
    MismatchedChainIds {
        grpc_url: String,
        expected: String,
        actual: Option<String>,
    },
    #[error(transparent)]
    SanityQueryFailed { source: QueryError },
    #[error("Could not find Sei gas config for chain ID {chain_id} at {url}")]
    SeiGasConfigNotFound { chain_id: String, url: String },
}

/// Parse errors while interacting with chain data.
#[derive(thiserror::Error, Debug, Clone)]
pub enum ChainParseError {
    InvalidTimestamp {
        timestamp: String,
        txhash: String,
        source: <DateTime<Utc> as FromStr>::Err,
    },
    InvalidInstantiatedContract {
        address: String,
        txhash: String,
        source: AddressError,
    },
    InvalidCodeId {
        code_id: String,
        txhash: String,
        source: std::num::ParseIntError,
    },
    NoCodeIdFound {
        txhash: String,
    },
    NoInstantiatedContractFound {
        txhash: String,
    },
    TxFees {
        err: String,
    },
}

impl Display for ChainParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.fmt_helper(f, false)
    }
}

impl ChainParseError {
    fn fmt_helper(&self, f: &mut std::fmt::Formatter, _pretty: bool) -> std::fmt::Result {
        match self {
            ChainParseError::InvalidTimestamp {
                timestamp,
                txhash,
                source,
            } => {
                write!(
                    f,
                    "Could not parse timestamp {timestamp:?} from transaction {txhash}: {source:?}"
                )
            }
            ChainParseError::InvalidInstantiatedContract {
                address,
                txhash,
                source,
            } => {
                write!(f, "Invalid instantiate contract address {address:?} from transaction {txhash}: {source}")
            }
            ChainParseError::InvalidCodeId {
                code_id,
                txhash,
                source,
            } => {
                write!(
                    f,
                    "Invalid code ID {code_id:?} from transaction {txhash}: {source:?}"
                )
            }
            ChainParseError::NoCodeIdFound { txhash } => {
                write!(
                    f,
                    "No code ID found when expecting a store code response in transaction {txhash}"
                )
            }
            ChainParseError::NoInstantiatedContractFound { txhash } => {
                write!(f, "No instantiated contract found in transaction {txhash}")
            }
            ChainParseError::TxFees { err } => {
                write!(f, "TxFees {err}")
            }
        }
    }
}

/// An error that occurs while connecting to a Cosmos gRPC endpoint.
///
/// This could be the initial connection or sending a new query.
#[derive(thiserror::Error, Debug, Clone)]
pub enum ConnectionError {
    SanityCheckFailed {
        grpc_url: Arc<String>,
        source: tonic::Status,
    },
    QueryFailed {
        grpc_url: Arc<String>,
    },
    TimeoutQuery {
        grpc_url: Arc<String>,
    },
    TimeoutConnecting {
        grpc_url: Arc<String>,
    },
    NoHealthyFound,
}

impl Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.fmt_helper(f, false)
    }
}

impl ConnectionError {
    fn fmt_helper(&self, f: &mut std::fmt::Formatter, pretty: bool) -> std::fmt::Result {
        match self {
            ConnectionError::SanityCheckFailed { grpc_url, source } => {
                if pretty {
                    write!(f, "Sanity check failed: {}", pretty_status(source, true))
                } else {
                    write!(
                        f,
                        "Sanity check on connection to {grpc_url} failed with gRPC status {source}"
                    )
                }
            }
            ConnectionError::QueryFailed { grpc_url } => {
                if pretty {
                    f.write_str("Network error occurred while performing query")
                } else {
                    write!(
                        f,
                        "Network error occured while performing query to {grpc_url}"
                    )
                }
            }
            ConnectionError::TimeoutQuery { grpc_url } => {
                if pretty {
                    f.write_str("Timeout hit when querying blockchain node")
                } else {
                    write!(f, "Timeout hit when querying gRPC endpoint {grpc_url}")
                }
            }
            ConnectionError::TimeoutConnecting { grpc_url } => {
                if pretty {
                    f.write_str("Timeout hit when connecting to blockchain node")
                } else {
                    write!(f, "Timeout hit when connecting to gRPC endpoint {grpc_url}")
                }
            }
            ConnectionError::NoHealthyFound => f.write_str("No healthy nodes found"),
        }
    }
}

/// Error while parsing a [crate::ContractAdmin].
#[derive(thiserror::Error, Debug, Clone)]
#[error(
    "Invalid contract admin. Must be 'no-admin', 'sender', or a valid address. Received: {input:?}"
)]
pub struct ContractAdminParseError {
    pub input: String,
}

/// Errors that occur while querying the chain.
#[derive(thiserror::Error, Debug, Clone)]
pub struct QueryError {
    pub action: Action,
    pub builder: Arc<CosmosBuilder>,
    pub height: Option<u64>,
    pub query: QueryErrorDetails,
    pub grpc_url: Arc<String>,
    pub node_health: NodeHealthReport,
}

impl QueryError {
    fn fmt_helper(&self, f: &mut std::fmt::Formatter, pretty: bool) -> std::fmt::Result {
        let QueryError {
            action,
            builder: _,
            height,
            query,
            grpc_url,
            node_health,
        } = self;
        if pretty {
            query.fmt_helper(f, true)?;
            f.write_str(" during ")?;
            action.fmt_helper(f, true)
        } else {
            write!(f, "On connection to {grpc_url}, while performing:\n{action}\n{query}\nHeight set to: {height:?}\n{node_health}")
        }
    }
}

impl Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.fmt_helper(f, false)
    }
}

/// General errors while interacting with the chain
///
/// This error type is used by the majority of the codebase. The idea is that
/// the other error types will represent "preparation" errors, and this will
/// represent errors during normal interaction.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    JsonSerialize(#[from] serde_json::Error),
    JsonDeserialize {
        source: serde_json::Error,
        action: Box<Action>,
    },
    Query(#[from] QueryError),
    ChainParse {
        source: Box<crate::error::ChainParseError>,
        action: Box<Action>,
    },
    InvalidChainResponse {
        message: String,
        action: Box<Action>,
    },
    WaitForTransactionTimedOut {
        txhash: String,
    },
    WaitForTransactionTimedOutWhile {
        txhash: String,
        action: Box<Action>,
    },
    LoadingWasmFromFile {
        path: PathBuf,
        source: std::io::Error,
    },
    TransactionFailed {
        code: CosmosSdkError,
        txhash: String,
        raw_log: String,
        action: Arc<Action>,
        grpc_url: Arc<String>,
        stage: TransactionStage,
    },
    Connection(#[from] ConnectionError),
    WasmGzipFailed {
        source: std::io::Error,
    },
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.fmt_helper(f, false)
    }
}

impl Error {
    fn fmt_helper(&self, f: &mut std::fmt::Formatter, pretty: bool) -> std::fmt::Result {
        match self {
            Error::JsonSerialize(e) => write!(f, "Unable to serialize value to JSON: {e}"),
            Error::JsonDeserialize { source, action } => {
                write!(
                    f,
                    "Unable to deserialize value from JSON while performing: "
                )?;
                action.fmt_helper(f, pretty)?;
                write!(f, ". Parse error: {source}")
            }
            Error::Query(e) => e.fmt_helper(f, pretty),
            Error::ChainParse { source, action } => {
                write!(f, "Error parsing data returned from chain: ")?;
                source.fmt_helper(f, pretty)?;
                write!(f, ". While performing: ")?;
                action.fmt_helper(f, pretty)
            }
            Error::InvalidChainResponse { message, action } => {
                write!(
                    f,
                    "Invalid response from chain: {message}. While performing: "
                )?;
                action.fmt_helper(f, pretty)
            }
            Error::WaitForTransactionTimedOut { txhash } => {
                write!(f, "Timed out waiting for transaction {txhash}")
            }
            Error::WaitForTransactionTimedOutWhile { txhash, action } => {
                write!(f, "Timed out waiting for transaction {txhash} during ")?;
                action.fmt_helper(f, pretty)
            }
            Error::LoadingWasmFromFile { path, source } => {
                write!(
                    f,
                    "Unable to load WASM code from {}: {source}",
                    path.display()
                )
            }
            Error::TransactionFailed {
                code,
                txhash,
                raw_log,
                action,
                grpc_url,
                stage,
            } => {
                if pretty {
                    write!(f, "Transaction {txhash} failed during {stage} with {code} and log: {raw_log} during ")?;
                    action.fmt_helper(f, true)
                } else {
                    write!(f, "Transaction {txhash} failed (on {grpc_url}) during {stage} with {code} and log: {raw_log}. Action: {action}.")
                }
            }
            Error::Connection(e) => e.fmt_helper(f, pretty),
            Error::WasmGzipFailed { source } => {
                write!(f, "Error during wasm Gzip compression: {source}")
            }
        }
    }

    pub(crate) fn get_sequence_mismatch_status(&self) -> Option<tonic::Status> {
        match self {
            Error::Query(QueryError {
                query: QueryErrorDetails::AccountSequenceMismatch(status),
                ..
            }) => Some(status.clone()),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum TransactionStage {
    Broadcast,
    Wait,
}

impl Display for TransactionStage {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            TransactionStage::Broadcast => "broadcast",
            TransactionStage::Wait => "wait",
        })
    }
}

/// The action being performed when an error occurred.
#[derive(Debug, Clone)]
pub enum Action {
    GetBaseAccount(Address),
    QueryAllBalances(Address),
    QueryGranterGrants(Address),
    CodeInfo(u64),
    GetTransactionBody(String),
    ListTransactionsFor(Address),
    GetBlock(i64),
    GetLatestBlock,
    Simulate(TxBuilder),
    Broadcast {
        txbuilder: TxBuilder,
        gas_wanted: u64,
        fee: cosmos_sdk_proto::cosmos::base::v1beta1::Coin,
    },
    WaitForBroadcast {
        txbuilder: TxBuilder,
        txhash: String,
    },
    RawQuery {
        contract: Address,
        key: StringOrBytes,
    },
    SmartQuery {
        contract: Address,
        message: StringOrBytes,
    },
    ContractInfo(Address),
    ContractHistory(Address),
    GetEarliestBlock,
    WaitForTransaction(String),
    OsmosisEpochsInfo,
    OsmosisTxFeesInfo,
    StoreCode {
        txbuilder: TxBuilder,
        txhash: String,
    },
    InstantiateContract {
        txbuilder: TxBuilder,
        txhash: String,
    },
    TokenFactory {
        txbuilder: TxBuilder,
        txhash: String,
    },
    BroadcastRaw,
    QueryTransactions {
        query: String,
        limit: Option<u64>,
        page: Option<u64>,
        order_by: OrderBy,
    },
}

impl Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.fmt_helper(f, false)
    }
}

impl Action {
    fn fmt_helper(&self, f: &mut std::fmt::Formatter, pretty: bool) -> std::fmt::Result {
        match self {
            Action::GetBaseAccount(address) => write!(f, "get base account {address}"),
            Action::QueryAllBalances(address) => write!(f, "query all balances for {address}"),
            Action::QueryGranterGrants(address) => write!(f, "query granter grants for {address}"),
            Action::CodeInfo(code_id) => write!(f, "get code info for code ID {code_id}"),
            Action::GetTransactionBody(txhash) => write!(f, "get transaction {txhash}"),
            Action::ListTransactionsFor(address) => write!(f, "list transactions for {address}"),
            Action::GetBlock(height) => write!(f, "get block {height}"),
            Action::GetLatestBlock => f.write_str("get latest block"),
            Action::Simulate(txbuilder) => write!(f, "simulating transaction: {txbuilder}"),
            Action::Broadcast {
                txbuilder,
                gas_wanted,
                fee,
            } => {
                if pretty {
                    write!(f, "broadcasting transaction")
                } else {
                    write!(
                        f,
                        "broadcasting transaction with {gas_wanted} gas and {}{} fee: {txbuilder}",
                        fee.amount, fee.denom
                    )
                }
            }
            Action::RawQuery { contract, key } => {
                write!(f, "raw query contract {contract} with key: {key}")
            }
            Action::SmartQuery { contract, message } => {
                write!(f, "smart query contract {contract} with message: {message}")
            }
            Action::ContractInfo(address) => write!(f, "contract info for {address}"),
            Action::ContractHistory(address) => write!(f, "contract history for {address}"),
            Action::GetEarliestBlock => f.write_str("get earliest block"),
            Action::WaitForTransaction(txhash) => write!(f, "wait for transaction {txhash}"),
            Action::OsmosisEpochsInfo => f.write_str("get Osmosis epochs info"),
            Action::OsmosisTxFeesInfo => f.write_str("get Osmosis txfees info"),
            Action::StoreCode { txbuilder, txhash } => {
                if pretty {
                    write!(f, "store code in {txhash}")
                } else {
                    write!(f, "store code in {txhash}: {txbuilder}")
                }
            }
            Action::InstantiateContract { txbuilder, txhash } => {
                if pretty {
                    write!(f, "instantiate contract in {txhash}")
                } else {
                    write!(f, "instantiate contract in {txhash}: {txbuilder}")
                }
            }
            Action::TokenFactory { txbuilder, txhash } => {
                if pretty {
                    write!(f, "perform token factory operation in {txhash}")
                } else {
                    write!(
                        f,
                        "perform token factory operation in {txhash}: {txbuilder}"
                    )
                }
            }
            Action::BroadcastRaw => f.write_str("broadcasting a raw transaction"),
            Action::WaitForBroadcast { txbuilder, txhash } => {
                if pretty {
                    write!(f, "waiting for transaction {txhash}")
                } else {
                    write!(f, "waiting for transaction {txhash} to land: {txbuilder}")
                }
            }
            Action::QueryTransactions { query, limit, page, order_by } => write!(f, "querying transactions for {query:?}, limit={limit:?}, page={page:?}, order={order_by:?}"),
        }
    }
}

/// A helper type to display either as UTF8 data or the underlying bytes
#[derive(Debug, Clone)]
pub struct StringOrBytes(pub Vec<u8>);

impl From<Vec<u8>> for StringOrBytes {
    fn from(value: Vec<u8>) -> Self {
        StringOrBytes(value)
    }
}

impl Display for StringOrBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match std::str::from_utf8(&self.0) {
            Ok(s) => f.write_str(s),
            Err(_) => write!(f, "{:?}", self.0),
        }
    }
}

/// The lower-level details of how a query failed.
///
/// This error type should generally be wrapped up in [QueryError] to provide
/// additional context.
#[derive(thiserror::Error, Debug, Clone)]
pub enum QueryErrorDetails {
    Unknown(tonic::Status),
    QueryTimeout(Duration),
    ConnectionError(ConnectionError),
    NotFound(String),
    CosmosSdk {
        error_code: CosmosSdkError,
        source: tonic::Status,
    },
    JsonParseError(tonic::Status),
    FailedToExecute(tonic::Status),
    HeightNotAvailable {
        lowest_height: Option<i64>,
        source: tonic::Status,
    },
    Unavailable {
        source: tonic::Status,
        status: http::status::StatusCode,
    },
    Unimplemented {
        source: tonic::Status,
    },
    TransportError {
        source: tonic::Status,
    },
    BlocksLagDetected {
        old_height: i64,
        new_height: i64,
        block_lag_allowed: u32,
    },
    NoNewBlockFound {
        age: Duration,
        age_allowed: Duration,
        old_height: i64,
        new_height: i64,
    },
    AccountSequenceMismatch(tonic::Status),
    RateLimited {
        source: tonic::Status,
    },
    Forbidden {
        source: tonic::Status,
    },
    NotGrpc {
        source: tonic::Status,
    },
}

impl Display for QueryErrorDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.fmt_helper(f, false)
    }
}

impl QueryErrorDetails {
    fn fmt_helper(&self, f: &mut std::fmt::Formatter, pretty: bool) -> std::fmt::Result {
        match self {
            QueryErrorDetails::Unknown(e) => {
                write!(
                    f,
                    "Unknown gRPC status returned: {}",
                    pretty_status(e, pretty)
                )
            }
            QueryErrorDetails::QueryTimeout(e) => {
                write!(f, "Query timed out after: {e:?}")
            }
            QueryErrorDetails::ConnectionError(e) => e.fmt_helper(f, pretty),
            QueryErrorDetails::NotFound(e) => {
                write!(f, "Not found returned from chain: {e}")
            }
            QueryErrorDetails::CosmosSdk { error_code, source } => {
                write!(
                    f,
                    "Cosmos SDK error code {error_code} returned: {}",
                    pretty_status(source, pretty)
                )
            }
            QueryErrorDetails::JsonParseError(e) => {
                write!(
                    f,
                    "Error parsing message into expected type: {}",
                    pretty_status(e, pretty)
                )
            }
            QueryErrorDetails::FailedToExecute(e) => {
                write!(f, "Failed to execute message: {}", pretty_status(e, pretty))
            }
            QueryErrorDetails::HeightNotAvailable {
                lowest_height,
                source,
            } => {
                write!(
                    f,
                    "Requested height not available, lowest height reported: {lowest_height:?}. {}",
                    pretty_status(source, pretty)
                )
            }
            QueryErrorDetails::Unavailable { source, status } => {
                write!(
                    f,
                    "Error querying server, received HTTP status code {status}. {}",
                    pretty_status(source, pretty)
                )
            }
            QueryErrorDetails::Unimplemented { source } => {
                write!(f, "Server does not implement expected services, it may not be a Cosmos gRPC endpoint. {}",pretty_status(source,pretty))
            }
            QueryErrorDetails::TransportError { source } => {
                write!(
                    f,
                    "Transport error with gRPC endpoint. {}",
                    pretty_status(source, pretty)
                )
            }
            QueryErrorDetails::BlocksLagDetected {
                old_height,
                new_height,
                block_lag_allowed,
            } => {
                write!(f, "Block lag detected. Previously saw {old_height}, but just received {new_height}. Allowed lag is {block_lag_allowed}.")
            }
            QueryErrorDetails::NoNewBlockFound {
                age,
                age_allowed,
                old_height,
                new_height,
            } => {
                write!(f, "No new block time found in {}s ({}s allowed). Old height: {old_height}. New height: {new_height}.", age.as_secs(), age_allowed.as_secs())
            }
            QueryErrorDetails::AccountSequenceMismatch(e) => {
                write!(f, "Account sequence mismatch: {}", pretty_status(e, pretty))
            }
            QueryErrorDetails::RateLimited { source } => {
                write!(
                    f,
                    "You appear to be rate limited by the gRPC server: {}",
                    pretty_status(source, pretty)
                )
            }
            QueryErrorDetails::Forbidden { source } => {
                write!(
                    f,
                    "The gRPC server is returning a 'forbidden' response: {}",
                    pretty_status(source, pretty)
                )
            }
            QueryErrorDetails::NotGrpc { source } => {
                write!(
                    f,
                    "Server returned response that does not look like valid gRPC: {}",
                    pretty_status(source, pretty)
                )
            }
        }
    }
}

/// Different known Cosmos SDK error codes
///
/// We can expand this over time, just including the most common ones for now
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum CosmosSdkError {
    /// Code 4
    Unauthorized,
    /// Code 5
    InsufficientFunds,
    /// Code 11
    OutOfGas,
    /// Code 13
    InsufficientFee,
    /// Code 19
    TxInMempool,
    /// Code 21
    TxTooLarge,
    /// Code 28
    InvalidChainId,
    /// Code 30
    TxTimeoutHeight,
    /// Code 32
    IncorrectAccountSequence,
    /// Codespace mempool, Code 3
    TxInCache,
    /// Some other error code
    Other { code: u32, codespace: String },
}

impl Display for CosmosSdkError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CosmosSdkError::Unauthorized => f.write_str("unauthorized (4)"),
            CosmosSdkError::InsufficientFunds => f.write_str("insufficient funds (5)"),
            CosmosSdkError::OutOfGas => f.write_str("out of gas (11)"),
            CosmosSdkError::InsufficientFee => f.write_str("insufficient fee (13)"),
            CosmosSdkError::TxInMempool => f.write_str("tx already in mempool (19)"),
            CosmosSdkError::TxInCache => f.write_str("tx already in cache (mempool:3)"),
            CosmosSdkError::TxTooLarge => f.write_str("tx too large (21)"),
            CosmosSdkError::InvalidChainId => f.write_str("invalid chain ID (28)"),
            CosmosSdkError::TxTimeoutHeight => f.write_str("tx timeout height (30)"),
            CosmosSdkError::IncorrectAccountSequence => {
                f.write_str("incorrect account sequence (32)")
            }
            CosmosSdkError::Other { code, codespace } => {
                write!(f, "code {code} (codespace {codespace})")
            }
        }
    }
}

impl CosmosSdkError {
    pub fn from_code(code: u32, codespace: &str) -> CosmosSdkError {
        if codespace == "sdk" {
            match code {
                4 => Self::Unauthorized,
                5 => Self::InsufficientFunds,
                11 => Self::OutOfGas,
                13 => Self::InsufficientFee,
                19 => Self::TxInMempool,
                21 => Self::TxTooLarge,
                28 => Self::InvalidChainId,
                30 => Self::TxTimeoutHeight,
                32 => Self::IncorrectAccountSequence,
                _ => Self::Other {
                    code,
                    codespace: codespace.to_owned(),
                },
            }
        } else if codespace == "mempool" && code == 3 {
            Self::TxInCache
        } else {
            Self::Other {
                code,
                codespace: codespace.to_owned(),
            }
        }
    }

    /// Do we consider a broadcast successful?
    pub(crate) fn is_successful_broadcast(&self) -> bool {
        match self {
            CosmosSdkError::TxInMempool | CosmosSdkError::TxInCache => true,
            CosmosSdkError::Unauthorized
            | CosmosSdkError::InsufficientFunds
            | CosmosSdkError::OutOfGas
            | CosmosSdkError::InsufficientFee
            | CosmosSdkError::TxTooLarge
            | CosmosSdkError::InvalidChainId
            | CosmosSdkError::TxTimeoutHeight
            | CosmosSdkError::IncorrectAccountSequence
            | CosmosSdkError::Other {
                code: _,
                codespace: _,
            } => false,
        }
    }
}

pub(crate) enum QueryErrorCategory {
    /// Should retry, kill the connection
    NetworkIssue,
    /// Don't retry, connection is fine
    ConnectionIsFine,
    /// No idea, make a test query and try again
    Unsure,
}

impl QueryErrorDetails {
    /// Indicates that the error may be transient and deserves a retry.
    pub(crate) fn error_category(&self) -> QueryErrorCategory {
        use QueryErrorCategory::*;
        match self {
            // Not sure, so give it a retry
            QueryErrorDetails::Unknown(_) => Unsure,
            // Same here, maybe it was a bad connection.
            QueryErrorDetails::QueryTimeout(_) => NetworkIssue,
            // Also possibly a bad connection
            QueryErrorDetails::ConnectionError(_) => NetworkIssue,
            QueryErrorDetails::NotFound(_) => ConnectionIsFine,
            QueryErrorDetails::CosmosSdk { error_code, .. } => {
                match *error_code {
                    // Treat account sequence issue as a transitent issue
                    CosmosSdkError::IncorrectAccountSequence => ConnectionIsFine,
                    // Invalid chain ID, we should try a different node if possible
                    CosmosSdkError::InvalidChainId => NetworkIssue,
                    _ => ConnectionIsFine,
                }
            }
            QueryErrorDetails::JsonParseError(_) => ConnectionIsFine,
            QueryErrorDetails::FailedToExecute(_) => ConnectionIsFine,
            // Interesting case here... maybe we need to treat it as a network
            // issue so we retry with a fallback node. Or maybe apps that need
            // that specific case handled should implement their own fallback
            // logic.
            QueryErrorDetails::HeightNotAvailable { .. } => ConnectionIsFine,
            QueryErrorDetails::Unavailable { .. } => NetworkIssue,
            QueryErrorDetails::Unimplemented { .. } => NetworkIssue,
            QueryErrorDetails::TransportError { .. } => NetworkIssue,
            QueryErrorDetails::BlocksLagDetected { .. } => NetworkIssue,
            QueryErrorDetails::NoNewBlockFound { .. } => NetworkIssue,
            // Same logic as CosmosSdk IncorrectAccountSequence above
            QueryErrorDetails::AccountSequenceMismatch { .. } => ConnectionIsFine,
            QueryErrorDetails::RateLimited { .. } => NetworkIssue,
            QueryErrorDetails::Forbidden { .. } => NetworkIssue,
            QueryErrorDetails::NotGrpc { .. } => NetworkIssue,
        }
    }

    pub(crate) fn from_tonic_status(err: tonic::Status) -> QueryErrorDetails {
        // For some reason, it looks like Osmosis testnet isn't returning a NotFound. Ugly workaround...
        if err.message().contains("not found") || err.code() == tonic::Code::NotFound {
            return QueryErrorDetails::NotFound(err.message().to_owned());
        }

        if err.code() == tonic::Code::Unavailable {
            let http = err.clone().into_http();
            return QueryErrorDetails::Unavailable {
                source: err,
                status: http.status(),
            };
        }

        if err.code() == tonic::Code::Unimplemented {
            return QueryErrorDetails::Unimplemented { source: err };
        }

        if let Some(source) = std::error::Error::source(&err) {
            if source.downcast_ref::<tonic::transport::Error>().is_some() {
                return QueryErrorDetails::TransportError { source: err };
            }
        }

        if let Some(error_code) = extract_cosmos_sdk_error_code(err.message()) {
            return QueryErrorDetails::CosmosSdk {
                error_code: CosmosSdkError::from_code(error_code, "wasm"),
                source: err,
            };
        }

        if err.message().starts_with("Error parsing into type ") {
            return QueryErrorDetails::JsonParseError(err);
        }

        if err.message().starts_with("failed to execute message;") {
            return QueryErrorDetails::FailedToExecute(err);
        }

        // This seems like a duplicate of Cosmos SDK error code 32. However,
        // this sometimes happens during the simulate step instead of broadcast,
        // in which case we don't get the error code. In theory, we could simply
        // generate a 32 error code value, but keeping it as a separate variant
        // just in case we need to distinguish the cases.
        if err.message().starts_with("account sequence mismatch") {
            return QueryErrorDetails::AccountSequenceMismatch(err);
        }

        if err.message().contains("status: 429") {
            return QueryErrorDetails::RateLimited { source: err };
        }
        if err.message().contains("status: 403") {
            return QueryErrorDetails::Forbidden { source: err };
        }

        if let Some(lowest_height) = get_lowest_height(err.message()) {
            return QueryErrorDetails::HeightNotAvailable {
                lowest_height: Some(lowest_height),
                source: err,
            };
        }

        if err.message().contains("status: 405") {
            return QueryErrorDetails::NotGrpc { source: err };
        }

        if err.message().contains("invalid compression flag") {
            return QueryErrorDetails::NotGrpc { source: err };
        }

        QueryErrorDetails::Unknown(err)
    }

    pub(crate) fn is_blocked(&self) -> bool {
        match self {
            QueryErrorDetails::Unknown(_)
            | QueryErrorDetails::QueryTimeout(_)
            | QueryErrorDetails::ConnectionError(_)
            | QueryErrorDetails::NotFound(_)
            | QueryErrorDetails::CosmosSdk { .. }
            | QueryErrorDetails::JsonParseError(_)
            | QueryErrorDetails::FailedToExecute(_)
            | QueryErrorDetails::HeightNotAvailable { .. }
            | QueryErrorDetails::Unavailable { .. }
            | QueryErrorDetails::Unimplemented { .. }
            | QueryErrorDetails::TransportError { .. }
            | QueryErrorDetails::BlocksLagDetected { .. }
            | QueryErrorDetails::NoNewBlockFound { .. }
            | QueryErrorDetails::AccountSequenceMismatch(_)
            | QueryErrorDetails::NotGrpc { .. } => false,
            QueryErrorDetails::RateLimited { .. } | QueryErrorDetails::Forbidden { .. } => true,
        }
    }
}

fn get_lowest_height(message: &str) -> Option<i64> {
    let per_needle = |needle: &str| {
        let trimmed = message.split(needle).nth(1)?.trim();
        let stripped = trimmed.strip_suffix(')').unwrap_or(trimmed);
        stripped.parse().ok()
    };
    for needle in ["lowest height is", "base height: "] {
        if let Some(x) = per_needle(needle) {
            return Some(x);
        }
    }
    None
}

fn extract_cosmos_sdk_error_code(message: &str) -> Option<u32> {
    message
        .strip_prefix("codespace wasm code ")?
        .split_once(':')?
        .0
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_success() {
        assert_eq!(
            extract_cosmos_sdk_error_code("codespace wasm code 9: query wasm contract failed: Error parsing into type foo::QueryMsg: unknown variant `{\"invalid_request\":{}}`, expected one of `bar`, `baz`"),
            Some(9)
        );
    }

    #[test]
    fn test_extract_fail() {
        assert_eq!(
            extract_cosmos_sdk_error_code("invalid Bech32 prefix; expected osmo, got inj"),
            None
        );
        assert_eq!(
            extract_cosmos_sdk_error_code("Error parsing into type foo::QueryMsg: unknown variant `{\"invalid_request\":{}}`, expected one of `version`, `bin`: query wasm contract failed"),
            None

        );
    }
}

#[derive(Clone, Debug)]
pub struct NodeHealthReport {
    pub nodes: Vec<SingleNodeHealthReport>,
}

#[derive(Clone, Debug)]
pub struct SingleNodeHealthReport {
    pub grpc_url: Arc<String>,
    pub is_fallback: bool,
    pub node_health_level: NodeHealthLevel,
    pub last_error: Option<LastNodeError>,
    pub error_count: usize,
    pub first_request: Option<DateTime<Utc>>,
    pub total_query_count: u64,
    pub total_error_count: u64,
}

/// Describes the health status of an individual node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodeHealthLevel {
    /// Not currently blocked, returns active error count
    Unblocked { error_count: usize },
    /// Do not use at all, such as during rate limiting
    Blocked,
}

impl Display for NodeHealthLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            NodeHealthLevel::Unblocked { error_count } => match error_count {
                0 => f.write_str("Healthy"),
                1 => f.write_str("1 error"),
                _ => write!(f, "{error_count} errors"),
            },
            NodeHealthLevel::Blocked => f.write_str("Blocked"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LastNodeError {
    pub timestamp: DateTime<Utc>,
    pub age: std::time::Duration,
    pub error: Arc<String>,
}

impl Display for NodeHealthReport {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for node in &self.nodes {
            writeln!(f, "{node}")?;
        }
        Ok(())
    }
}

impl Display for SingleNodeHealthReport {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "Health report for {}. Fallback: {}. Health: {}. ",
            self.grpc_url, self.is_fallback, self.node_health_level
        )?;
        match &self.last_error {
            None => write!(f, "No errors")?,
            Some(LastNodeError {
                timestamp,
                age,
                error,
            }) => write!(f, "Last error: {timestamp} ({age:?}): {error}")?,
        }
        if let Some(first_request) = self.first_request {
            let since = (Utc::now() - first_request).num_minutes();

            enum ConversionError {
                Overflow,
                DivideByZero,
            }

            let rate_per_minute = (|| {
                let since = u64::try_from(since).map_err(|_| ConversionError::Overflow)?;
                self.total_query_count
                    .checked_div(since)
                    .ok_or(ConversionError::DivideByZero)
                    .map(|item| item.to_string())
            })()
            .unwrap_or_else(|err| match err {
                ConversionError::Overflow => "Overflow when converting since".to_owned(),
                ConversionError::DivideByZero => "since is 0".to_owned(),
            });
            let errors_per_minute = (|| {
                let since = u64::try_from(since).map_err(|_| ConversionError::Overflow)?;
                self.total_error_count
                    .checked_div(since)
                    .ok_or(ConversionError::DivideByZero)
                    .map(|item| item.to_string())
            })()
            .unwrap_or_else(|err| match err {
                ConversionError::Overflow => "Overflow when converting since".to_owned(),
                ConversionError::DivideByZero => "since is 0".to_owned(),
            });

            write!(
                f,
                ". First request: {} (Since {} minutes). Total queries: {} (RPM: {}). Total errors: {} (RPM: {})",
                first_request, since, self.total_query_count, rate_per_minute, self.total_error_count, errors_per_minute
            )?;
        }
        Ok(())
    }
}

/// Errors that can occur while getting the first block.
#[derive(thiserror::Error, Debug)]
pub enum FirstBlockAfterError {
    #[error(transparent)]
    CosmosError(#[from] Error),
    #[error(
        "No blocks exist before {timestamp}, earliest block is {earliest_height} @ {earliest_timestamp}"
    )]
    NoBlocksExistBefore {
        timestamp: DateTime<Utc>,
        earliest_height: i64,
        earliest_timestamp: DateTime<Utc>,
    },
    #[error(
        "No blocks exist after {timestamp}, latest block is {latest_height} @ {latest_timestamp}"
    )]
    NoBlocksExistAfter {
        timestamp: DateTime<Utc>,
        latest_height: i64,
        latest_timestamp: DateTime<Utc>,
    },
}

impl Error {
    /// Wrap up in a [PrettyError].
    pub fn pretty(self) -> PrettyError {
        PrettyError { source: self }
    }
}

/// Provide a user-friendly version of the error messages.
///
/// Normal display of errors messages is intended for server-side debugging. This contains more information than we would normally want for user-facing messages. This method provides an alternative.
#[derive(Debug, thiserror::Error)]
pub struct PrettyError {
    pub source: Error,
}

impl Display for PrettyError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.source.fmt_helper(f, true)
    }
}

fn pretty_status(status: &tonic::Status, pretty: bool) -> PrettyStatus {
    PrettyStatus(status, pretty)
}

struct PrettyStatus<'a>(&'a tonic::Status, bool);

impl Display for PrettyStatus<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.1 {
            write!(f, "{}", self.0.message())
        } else {
            write!(f, "{}", self.0)
        }
    }
}
