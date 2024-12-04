//! Provides helpers for generating Cosmos values from command line parameters.

use std::{path::PathBuf, str::FromStr};

use crate::{error::BuilderError, AddressHrp, Cosmos, CosmosBuilder, CosmosNetwork};

/// Command line options for connecting to a Cosmos network
#[derive(clap::Parser, Clone, Debug)]
pub struct CosmosOpt {
    /// Which blockchain to connect to for grabbing blocks
    #[clap(long, env = "COSMOS_NETWORK", global = true)]
    pub network: Option<String>,
    #[cfg(feature = "config")]
    /// Config file for network default overrides
    #[clap(long, env = "COSMOS_CONFIG", global = true)]
    pub config: Option<PathBuf>,
    #[cfg(feature = "config")]
    /// Disable usage of config file overrides
    #[clap(long, env = "COSMOS_CONFIG_DISABLE", global = true)]
    pub config_disable: bool,
    /// Optional gRPC endpoint override
    #[clap(long, env = "COSMOS_GRPC", global = true)]
    pub cosmos_grpc: Option<String>,
    /// Optional gRPC fallback endpoints
    #[clap(
        long,
        env = "COSMOS_GRPC_FALLBACKS",
        global = true,
        value_delimiter = ','
    )]
    pub cosmos_grpc_fallbacks: Vec<String>,
    /// Optional chain ID override
    #[clap(long, env = "COSMOS_CHAIN_ID", global = true)]
    pub chain_id: Option<String>,
    /// Optional gas multiplier override
    #[clap(long, env = "COSMOS_GAS_MULTIPLIER", global = true)]
    pub gas_multiplier: Option<f64>,
    /// Referer header
    #[clap(long, short, global = true, env = "COSMOS_REFERER_HEADER")]
    referer_header: Option<String>,
    /// Gas coin (e.g. uosmo)
    #[clap(long, global = true, env = "COSMOS_GAS_COIN")]
    gas_coin: Option<String>,
    /// Human readable part (HRP) of wallet addresses
    #[clap(long, global = true, env = "COSMOS_HRP")]
    hrp: Option<AddressHrp>,
    /// Query timeout in seconds
    #[clap(long, global = true, env = "COSMOS_QUERY_TIMEOUT_SECONDS")]
    query_timeout_seconds: Option<u32>,
}

/// Errors for working with [CosmosOpt]
#[derive(thiserror::Error, Debug)]
#[allow(missing_docs)]
pub enum CosmosOptError {
    #[error("No network specified, either provide the COSMOS_NETWORK env var or --network option, or provide the following settings: {missing}")]
    NoNetworkProvided { missing: String },
    #[error("{source}")]
    CosmosBuilderError { source: BuilderError },
    #[error(transparent)]
    NetworkParseError {
        #[from]
        source: strum::ParseError,
    },
    #[cfg(feature = "config")]
    #[error(transparent)]
    ConfigError {
        #[from]
        source: crate::CosmosConfigError,
    },
}

impl CosmosOpt {
    /// Convert these options into a new [CosmosBuilder].
    pub async fn into_builder(self) -> Result<CosmosBuilder, CosmosOptError> {
        let CosmosOpt {
            network,
            cosmos_grpc,
            cosmos_grpc_fallbacks,
            chain_id,
            gas_multiplier,
            referer_header,
            gas_coin,
            hrp,
            #[cfg(feature = "config")]
            config,
            #[cfg(feature = "config")]
            config_disable,
            query_timeout_seconds,
        } = self;

        // Do the error checking here instead of in clap so that the field can
        // be global.
        let mut builder = match network {
            Some(network) => {
                async fn builder_without_config(
                    network: &str,
                ) -> Result<CosmosBuilder, CosmosOptError> {
                    CosmosNetwork::from_str(network)
                        .map_err(|source| CosmosOptError::NetworkParseError { source })?
                        .builder()
                        .await
                        .map_err(|source| CosmosOptError::CosmosBuilderError { source })
                }
                #[cfg(feature = "config")]
                let mut builder = {
                    use crate::{CosmosConfig, CosmosConfigError};
                    if config_disable {
                        builder_without_config(&network).await?
                    } else {
                        match &config {
                            Some(config) => {
                                CosmosConfig::load_from(config, true)?
                                    .builder_for(&network)
                                    .await?
                            }
                            None => match CosmosConfig::load() {
                                Ok(config) => config.builder_for(&network).await?,
                                Err(e @ CosmosConfigError::ProjectDirsNotFound) => {
                                    tracing::warn!("{e}");
                                    builder_without_config(&network).await?
                                }
                                Err(e) => return Err(e.into()),
                            },
                        }
                    }
                };
                #[cfg(not(feature = "config"))]
                let mut builder = builder_without_config(&network).await?;
                if let Some(grpc) = cosmos_grpc {
                    builder.set_grpc_url(grpc);
                }
                if let Some(chain_id) = chain_id {
                    builder.set_chain_id(chain_id);
                }
                if let Some(gas_coin) = gas_coin {
                    builder.set_gas_coin(gas_coin);
                }
                if let Some(hrp) = hrp {
                    builder.set_hrp(hrp)
                }
                builder
            }
            None => {
                #[cfg(feature = "config")]
                if config.is_some() {
                    tracing::warn!(
                        "You provided a config file, but without a network name it will be ignored"
                    );
                }
                let mut missing = vec![];
                if cosmos_grpc.is_none() {
                    missing.push("COSMOS_GRPC");
                }
                if chain_id.is_none() {
                    missing.push("COSMOS_CHAIN_ID");
                }
                if gas_coin.is_none() {
                    missing.push("COSMOS_GAS_COIN");
                }
                if hrp.is_none() {
                    missing.push("COSMOS_HRP");
                }
                match (cosmos_grpc, chain_id, gas_coin, hrp) {
                    (Some(grpc), Some(chain_id), Some(gas_coin), Some(hrp)) => {
                        assert!(missing.is_empty());
                        CosmosBuilder::new(chain_id, gas_coin, hrp, grpc)
                    }
                    _ => {
                        assert!(!missing.is_empty());
                        return Err(CosmosOptError::NoNetworkProvided {
                            missing: missing.join(", "),
                        });
                    }
                }
            }
        };
        for fallback in cosmos_grpc_fallbacks {
            builder.add_grpc_fallback_url(fallback);
        }

        if let Some(gas_multiplier) = gas_multiplier {
            builder.set_gas_estimate_multiplier(gas_multiplier);
        }
        builder.set_referer_header(referer_header);
        builder.set_query_timeout_seconds(query_timeout_seconds);

        Ok(builder)
    }

    /// Convenient for calling [CosmosOpt::into_builder] and then [CosmosBuilder::build].
    pub async fn build(self) -> Result<Cosmos, CosmosOptError> {
        self.into_builder()
            .await?
            .build()
            .map_err(|source| CosmosOptError::CosmosBuilderError { source })
    }
}
