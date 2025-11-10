use std::{collections::BTreeMap, sync::Arc, time::Duration};

use crate::{
    gas_multiplier::{GasMultiplier, GasMultiplierConfig},
    gas_price::GasPriceMethod,
    AddressHrp, ContractType, DynamicGasMultiplier,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct OsmosisGasParams {
    pub(crate) low_multiplier: f64,
    pub(crate) high_multiplier: f64,
}

/// Used to build a [crate::Cosmos].
#[derive(Clone, Debug)]
pub struct CosmosBuilder {
    grpc_url: Arc<String>,
    grpc_fallback_urls: Vec<Arc<String>>,
    chain_id: String,
    gas_coin: String,
    hrp: AddressHrp,
    is_fast_chain: bool,

    // Values with defaults
    gas_estimate_multiplier: GasMultiplierConfig,
    pub(crate) gas_price_method: Option<GasPriceMethod>,
    gas_price_retry_attempts: Option<u64>,
    transaction_attempts: Option<usize>,
    referer_header: Option<String>,
    request_count: Option<usize>,
    connection_timeout: Option<Duration>,
    idle_timeout_seconds: Option<u32>,
    query_timeout_seconds: Option<u32>,
    query_retries: Option<usize>,
    block_lag_allowed: Option<u32>,
    latest_block_age_allowed: Option<Duration>,
    fallback_timeout: Option<Duration>,
    pub(crate) chain_paused_method: ChainPausedMethod,
    pub(crate) autofix_simulate_sequence_mismatch: Option<bool>,
    dynamic_gas_retries: Option<u32>,
    osmosis_gas_params: Option<OsmosisGasParams>,
    osmosis_gas_price_too_old_seconds: Option<u64>,
    max_price: Option<f64>,
    rate_limit_per_second: Option<u64>,
    log_requests: Option<bool>,
    max_decoding_message_size: Option<usize>,
    all_nodes_broadcast: bool,
    http2_keep_alive_interval: Option<Duration>,
    keep_alive_while_idle: Option<bool>,
    simulate_with_gas_coin: bool,
    delay_before_fallback: Option<tokio::time::Duration>,
    pub(crate) code_ids: BTreeMap<ContractType, u64>,
}

impl CosmosBuilder {
    /// Create a new [CosmosBuilder] with default options where possible.
    pub fn new(
        chain_id: impl Into<String>,
        gas_coin: impl Into<String>,
        hrp: AddressHrp,
        grpc_url: impl Into<String>,
    ) -> CosmosBuilder {
        let chain_id = chain_id.into();
        let simulate_with_gas_coin = chain_id == "pion-1";
        Self {
            grpc_url: Arc::new(grpc_url.into()),
            grpc_fallback_urls: vec![],
            chain_id,
            gas_coin: gas_coin.into(),
            hrp,
            gas_estimate_multiplier: GasMultiplierConfig::Default,
            gas_price_method: None,
            gas_price_retry_attempts: None,
            transaction_attempts: None,
            referer_header: None,
            request_count: None,
            connection_timeout: None,
            idle_timeout_seconds: None,
            query_timeout_seconds: None,
            query_retries: None,
            block_lag_allowed: None,
            latest_block_age_allowed: None,
            fallback_timeout: None,
            chain_paused_method: ChainPausedMethod::None,
            autofix_simulate_sequence_mismatch: None,
            dynamic_gas_retries: None,
            osmosis_gas_params: None,
            osmosis_gas_price_too_old_seconds: None,
            max_price: None,
            rate_limit_per_second: None,
            is_fast_chain: matches!(hrp.as_str(), "sei" | "inj"),
            log_requests: None,
            max_decoding_message_size: None,
            all_nodes_broadcast: true,
            http2_keep_alive_interval: None,
            keep_alive_while_idle: None,
            simulate_with_gas_coin,
            delay_before_fallback: None,
            code_ids: BTreeMap::new(),
        }
    }

    /// gRPC endpoint to connect to
    ///
    /// This is the primary endpoint, not any fallbacks provided
    pub fn grpc_url(&self) -> &str {
        self.grpc_url.as_ref()
    }

    pub(crate) fn grpc_url_arc(&self) -> &Arc<String> {
        &self.grpc_url
    }

    /// See [Self::grpc_url]
    pub fn set_grpc_url(&mut self, grpc_url: impl Into<String>) {
        self.grpc_url = grpc_url.into().into();
    }

    /// Add a fallback gRPC URL
    pub fn add_grpc_fallback_url(&mut self, url: impl Into<String>) {
        self.grpc_fallback_urls.push(url.into().into());
    }

    /// gRPC fallback URLs
    pub fn grpc_fallback_urls(&self) -> &Vec<Arc<String>> {
        &self.grpc_fallback_urls
    }

    /// Chain ID we want to communicate with
    pub fn chain_id(&self) -> &str {
        self.chain_id.as_ref()
    }

    /// See [Self::chain_id]
    pub fn set_chain_id(&mut self, chain_id: String) {
        self.chain_id = chain_id;
    }

    /// Native coin used for gas payments
    pub fn gas_coin(&self) -> &str {
        self.gas_coin.as_ref()
    }

    /// See [Self::gas_coin]
    pub fn set_gas_coin(&mut self, gas_coin: String) {
        self.gas_coin = gas_coin;
    }

    /// Human-readable part (HRP) of chain addresses
    pub fn hrp(&self) -> AddressHrp {
        self.hrp
    }

    /// See [Self::hrp]
    pub fn set_hrp(&mut self, hrp: AddressHrp) {
        self.hrp = hrp;
    }

    /// Set a code ID for the given contract type.
    pub fn set_code_id(&mut self, contract_type: ContractType, code_id: u64) {
        self.code_ids.insert(contract_type, code_id);
    }

    /// Revert to the default gas multiplier value (static value of 1.3).
    ///
    /// This value comes from CosmJS and OsmoJS:
    ///
    /// * <https://github.com/cosmos/cosmjs/blob/e8e65aa0c145616ccb58625c32bffe08b46ff574/packages/cosmwasm-stargate/src/signingcosmwasmclient.ts#L550>
    /// * <https://github.com/osmosis-labs/osmojs/blob/bacb2fc322abc3d438581f5dce049f5ae467059d/packages/osmojs/src/utils/gas/estimation.ts#L10>
    pub fn set_default_gas_estimate_multiplier(&mut self) {
        self.gas_estimate_multiplier = GasMultiplierConfig::Default;
    }

    pub(crate) fn build_gas_multiplier(&self) -> GasMultiplier {
        self.gas_estimate_multiplier.build()
    }

    /// Set a static gas multiplier to the given value.
    pub fn set_gas_estimate_multiplier(&mut self, gas_estimate_multiplier: f64) {
        self.gas_estimate_multiplier = GasMultiplierConfig::Static(gas_estimate_multiplier);
    }

    /// Set a dynamic gas multiplier.
    pub fn set_dynamic_gas_estimate_multiplier(&mut self, config: DynamicGasMultiplier) {
        self.gas_estimate_multiplier = GasMultiplierConfig::Dynamic(config);
    }

    /// How many times to retry a transaction with corrected gas multipliers.
    ///
    /// If you're using a dynamic gas estimate multiplier, this will indicate
    /// how many times we should retry a transaction after an "out of gas" before
    /// giving up. Intermediate errors will be logged with `tracing`. If you're not
    /// using dynamic gas, this option has no effect. If the gas multiplier reaches the
    /// maximum, not retry will occur.
    ///
    /// Default: 4
    pub fn get_dynamic_gas_retries(&self) -> u32 {
        self.dynamic_gas_retries.unwrap_or(4)
    }

    /// See [Self::get_dynamic_gas_retries]
    pub fn set_dynamic_gas_retries(&mut self, dynamic_gas_retries: Option<u32>) {
        self.dynamic_gas_retries = dynamic_gas_retries;
    }

    /// Set the lower and upper bounds of gas price.
    pub fn set_gas_price(&mut self, low: f64, high: f64) {
        self.gas_price_method = Some(GasPriceMethod::new_static(low, high));
    }

    pub(crate) fn set_gas_price_method(&mut self, method: GasPriceMethod) {
        self.gas_price_method = Some(method);
    }

    /// How many retries at different gas prices should we try before using high
    ///
    /// Default: 3
    ///
    /// If this is 0, we'll always go straight to high. 1 means we'll try the
    /// low and the high. 2 means we'll try low, midpoint, and high. And so on
    /// from there.
    pub fn gas_price_retry_attempts(&self) -> u64 {
        self.gas_price_retry_attempts.unwrap_or(3)
    }

    /// See [Self::gas_price_retry_attempts]
    pub fn set_gas_price_retry_attempts(&mut self, gas_price_retry_attempts: Option<u64>) {
        self.gas_price_retry_attempts = gas_price_retry_attempts;
    }

    /// How many attempts to give a transaction before giving up
    ///
    /// Default: 30
    pub fn transaction_attempts(&self) -> usize {
        self.transaction_attempts.unwrap_or(30)
    }

    /// See [Self::transaction_attempts]
    pub fn set_transaction_attempts(&mut self, transaction_attempts: Option<usize>) {
        self.transaction_attempts = transaction_attempts;
    }

    /// Referrer header sent to the server
    pub fn referer_header(&self) -> Option<&str> {
        self.referer_header.as_deref()
    }

    /// See [Self::referer_header]
    pub fn set_referer_header(&mut self, referer_header: Option<String>) {
        self.referer_header = referer_header;
    }

    /// The maximum number of concurrent requests
    ///
    /// This is a global limit for the generated [Cosmos], and will apply across all endpoints.
    ///
    /// Defaults to 128
    pub fn request_count(&self) -> usize {
        self.request_count.unwrap_or(128)
    }

    /// See [Self::request_count]
    pub fn set_request_count(&mut self, request_count: Option<usize>) {
        self.request_count = request_count;
    }

    /// See rate limit per second
    pub fn rate_limit(&self) -> Option<u64> {
        self.rate_limit_per_second
    }

    /// Set rate limit per second
    pub fn set_rate_limit(&mut self, limit: u64) {
        self.rate_limit_per_second = Some(limit);
    }

    /// Sets the duration to wait for a connection.
    ///
    /// Defaults to 5 seconds if there are no fallbacks, 1.2 seconds if there
    /// are.
    pub fn connection_timeout(&self) -> Duration {
        self.connection_timeout.unwrap_or_else(|| {
            if self.grpc_fallback_urls.is_empty() {
                Duration::from_secs(5)
            } else {
                Duration::from_millis(1200)
            }
        })
    }

    /// See [Self::connection_timeout]
    pub fn set_connection_timeout(&mut self, connection_timeout: Option<Duration>) {
        self.connection_timeout = connection_timeout;
    }

    /// Sets the number of seconds before an idle connection is reaped
    ///
    /// Defaults to 20 seconds
    pub fn idle_timeout_seconds(&self) -> u32 {
        self.idle_timeout_seconds.unwrap_or(20)
    }

    /// See [Self::idle_timeout_seconds]
    pub fn set_idle_timeout_seconds(&mut self, idle_timeout_seconds: Option<u32>) {
        self.idle_timeout_seconds = idle_timeout_seconds;
    }

    /// Sets the number of seconds before timing out a gRPC query
    ///
    /// Defaults to 5 seconds
    pub fn query_timeout_seconds(&self) -> u32 {
        self.query_timeout_seconds.unwrap_or(5)
    }

    /// See [Self::query_timeout_seconds]
    pub fn set_query_timeout_seconds(&mut self, query_timeout_seconds: Option<u32>) {
        self.query_timeout_seconds = query_timeout_seconds;
    }

    /// Number of attempts to make at a query before giving up.
    ///
    /// Only retries if there is a tonic-level error.
    ///
    /// Defaults to 3
    pub fn query_retries(&self) -> usize {
        self.query_retries.unwrap_or(3)
    }

    /// See [Self::query_retries]
    pub fn set_query_retries(&mut self, query_retries: Option<usize>) {
        self.query_retries = query_retries;
    }

    /// How many blocks a response is allowed to lag.
    ///
    /// Defaults to 10 for most chains, 50 for fast chains (currently: Sei and Injective).
    ///
    /// This is intended to detect when one of the nodes in a load balancer has
    /// stopped syncing while others are making progress.
    pub fn block_lag_allowed(&self) -> u32 {
        self.block_lag_allowed
            .unwrap_or(if self.is_fast_chain { 50 } else { 10 })
    }

    /// See [Self::block_lag_allowed]
    pub fn set_block_lag_allowed(&mut self, block_lag_allowed: Option<u32>) {
        self.block_lag_allowed = block_lag_allowed;
    }

    /// How long before we expect to see a new block
    ///
    /// Defaults to 60 seconds
    ///
    /// If we go this amount of time without seeing a new block, queries will
    /// fail on the assumption that they are getting stale data.
    pub fn latest_block_age_allowed(&self) -> Duration {
        self.latest_block_age_allowed
            .unwrap_or_else(|| Duration::from_secs(60))
    }

    /// See [Self::latest_block_age_allowed]
    pub fn set_latest_block_age_allowed(&mut self, latest_block_age_allowed: Option<Duration>) {
        self.latest_block_age_allowed = latest_block_age_allowed;
    }

    /// How long we allow a fallback connection to last before timing out.
    ///
    /// Defaults to 5 minutes.
    ///
    /// This forces systems to try to go back to the primary endpoint regularly.
    pub fn fallback_timeout(&self) -> Duration {
        self.fallback_timeout
            .unwrap_or_else(|| Duration::from_secs(300))
    }

    /// See [Self::fallback_timeout]
    pub fn set_fallback_timeout(&mut self, fallback_timeout: Option<Duration>) {
        self.fallback_timeout = fallback_timeout;
    }

    pub(crate) fn set_osmosis_mainnet_chain_paused(&mut self) {
        self.chain_paused_method = ChainPausedMethod::OsmosisMainnet;
    }

    /// Should we automatically retry transactions with corrected
    /// sequence numbers during simulating transaction ?
    ///
    /// Default: [true]
    pub fn autofix_sequence_mismatch(&self) -> bool {
        self.autofix_simulate_sequence_mismatch.unwrap_or(true)
    }

    /// See [Self::autofix_sequence_mismatch]
    pub fn set_autofix_sequence_mismatch(&mut self, autofix_sequence_mismatch: Option<bool>) {
        self.autofix_simulate_sequence_mismatch = autofix_sequence_mismatch;
    }

    /// Set parameters for Osmosis's EIP fee market gas.
    ///
    /// Low and high multiplier indicate how much to multiply the base fee by to get low and high prices, respectively. The max price is a cap on what those results will be.
    ///
    /// Defaults: 1.2, 10.0, and 0.01
    pub fn set_osmosis_gas_params(&mut self, low_multiplier: f64, high_multiplier: f64) {
        self.osmosis_gas_params = Some(OsmosisGasParams {
            low_multiplier,
            high_multiplier,
        });
    }

    /// Sets the maximum gas price to be used on Osmosis mainnet.
    pub fn set_max_gas_price(&mut self, max_price: f64) {
        self.max_price = Some(max_price);
    }

    pub(crate) fn get_osmosis_gas_params(&self) -> OsmosisGasParams {
        self.osmosis_gas_params.unwrap_or(OsmosisGasParams {
            low_multiplier: 1.2,
            high_multiplier: 10.0,
        })
    }

    pub(crate) fn get_init_max_gas_price(&self) -> f64 {
        self.max_price.unwrap_or(if self.chain_id() == "osmosis-1" {
            0.2
        } else {
            0.01
        })
    }

    /// How many seconds old the Osmosis gas price needs to be before we recheck.
    ///
    /// Default: 5 seconds
    pub fn get_osmosis_gas_price_too_old_seconds(&self) -> u64 {
        self.osmosis_gas_price_too_old_seconds.unwrap_or(5)
    }

    /// See [Self::get_osmosis_gas_price_too_old_seconds]
    pub fn set_osmosis_gas_price_too_old_seconds(&mut self, secs: u64) {
        self.osmosis_gas_price_too_old_seconds = Some(secs);
    }

    /// Should we log Cosmos requests made?
    ///
    /// Default: false
    pub fn get_log_requests(&self) -> bool {
        self.log_requests.unwrap_or_default()
    }

    /// See [Self::get_log_requests]
    pub fn set_log_requests(&mut self, log_requests: bool) {
        self.log_requests = Some(log_requests);
    }

    /// Limits the maximum size of a decoded message.
    ///
    /// Default: 16 MB
    pub fn get_max_decoding_message_size(&self) -> usize {
        self.max_decoding_message_size.unwrap_or(16 * 1024 * 1024)
    }

    /// See [Self::get_max_decoding_message_size]
    pub fn set_max_decoding_message_size(&mut self, max_decoding_message_size: usize) {
        self.max_decoding_message_size = Some(max_decoding_message_size);
    }

    /// When broadcasting transactions, should we also broadcast to all fallback nodes?
    ///
    /// This is intended to work around cases where broadcasting to the primary
    /// node is failing, but other kinds of queries are working.
    ///
    /// Default: [true]
    pub fn get_all_nodes_broadcast(&self) -> bool {
        self.all_nodes_broadcast
    }

    /// See [Self::get_all_nodes_broadcast]
    pub fn set_all_nodes_broadcast(&mut self, value: bool) {
        self.all_nodes_broadcast = value;
    }

    /// Sets an interval for HTTP2 Ping frames should be sent to keep
    /// a connection alive.
    ///
    /// Uses hyper’s default otherwise.
    pub fn set_http2_keep_alive_interval(&mut self, value: Duration) {
        self.http2_keep_alive_interval = Some(value)
    }

    /// See [Self::set_http2_keep_alive_interval]
    pub fn get_http2_keep_alive_interval(&self) -> Option<Duration> {
        self.http2_keep_alive_interval
    }

    /// Sets whether HTTP2 keep-alive should apply while the
    /// connection is idle.
    ///
    /// Uses hyper’s default otherwise.
    pub fn set_keep_alive_while_idle(&mut self) {
        self.keep_alive_while_idle = Some(true)
    }

    /// See [Self::set_keep_alive_while_idle]
    pub fn get_keep_alive_while_idle(&self) -> Option<bool> {
        self.keep_alive_while_idle
    }

    /// When simulating transactions, do we include a fee with the gas coin?
    ///
    /// Most networks do not require this. However, Neutron Testnet started requiring
    /// this some time around June 2024.
    pub fn get_simulate_with_gas_coin(&self) -> bool {
        self.simulate_with_gas_coin
    }

    /// See [Self::get_simulate_with_gas_coin]
    pub fn set_simulate_with_gas_coin(&mut self, value: bool) {
        self.simulate_with_gas_coin = value;
    }

    /// How long to delay between each fallback node attempt?
    ///
    /// Default: 500ms
    pub fn get_delay_before_fallback(&self) -> tokio::time::Duration {
        self.delay_before_fallback
            .unwrap_or(tokio::time::Duration::from_millis(500))
    }

    /// See [Self::get_delay_before_fallback]
    pub fn set_delay_before_fallback(&mut self, delay: tokio::time::Duration) {
        self.delay_before_fallback = Some(delay);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ChainPausedMethod {
    None,
    OsmosisMainnet,
}
