use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    str::FromStr,
};

use figment::{
    providers::{Env, Format, Toml},
    Figment,
};

use crate::{AddressHrp, CosmosBuilder, CosmosNetwork};

/// Configuration overrides for individual network
pub struct CosmosConfig {
    path: PathBuf,
    inner: CosmosConfigInner,
}

#[derive(serde::Deserialize)]
struct CosmosConfigInner {
    #[serde(default)]
    network: HashMap<String, NetworkConfig>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct NetworkConfig {
    grpc: Option<String>,
    chain_id: Option<String>,
    gas_coin: Option<String>,
    hrp: Option<AddressHrp>,
    #[serde(default)]
    grpc_fallbacks: Vec<String>,
}

impl NetworkConfig {
    fn apply_extra_config(&self, builder: &mut CosmosBuilder) {
        for fallback in &self.grpc_fallbacks {
            builder.add_grpc_fallback_url(fallback);
        }
    }
}

/// Errors which can occur while loading the config file.
#[derive(thiserror::Error, Debug)]
#[allow(missing_docs)]
pub enum CosmosConfigError {
    #[error("Config file not found: {}", path.display())]
    FileNotFound { path: PathBuf },
    #[error("Misconfiguration detected, unable to find your default config file location")]
    ProjectDirsNotFound,
    #[error("Error loading config file {}: {source}", path.display())]
    ConfigLoadError {
        source: figment::Error,
        path: PathBuf,
    },
    #[error("Unknown network {network:?} specified, not a known built-in network or found in config {}", config.display())]
    UnknownNetwork { network: String, config: PathBuf },
    #[error(transparent)]
    Builder { source: crate::error::BuilderError },
    #[error("Missing required config values for network {network:?} in config file {}: {missing}", path.display())]
    MissingRequiredConfig {
        missing: String,
        path: PathBuf,
        network: String,
    },
}

impl CosmosConfig {
    /// Load the config values from the default config file location
    pub fn load() -> Result<CosmosConfig, CosmosConfigError> {
        let dirs = directories::ProjectDirs::from("com", "fpco", "cosmos-rs")
            .ok_or(CosmosConfigError::ProjectDirsNotFound)?;
        let mut file = dirs.config_dir().to_owned();
        file.push("config.toml");
        Self::load_from(&file, false)
    }

    /// Load the config values from the specified file
    pub(crate) fn load_from(
        config: &Path,
        required: bool,
    ) -> Result<CosmosConfig, CosmosConfigError> {
        if required && !config.exists() {
            return Err(CosmosConfigError::FileNotFound {
                path: config.to_owned(),
            });
        }
        let inner = Figment::new()
            .merge(Toml::file(config))
            .merge(Env::prefixed("COSMOS_CONFIG_"))
            .extract()
            .map_err(|source| CosmosConfigError::ConfigLoadError {
                source,
                path: config.to_owned(),
            })?;
        Ok(CosmosConfig {
            path: config.to_owned(),
            inner,
        })
    }

    /// Generate a builder for the given network name
    ///
    /// If the network name is not a valid [CosmosNetwork], and there are insufficient config settings in the config file, this will generate an error.
    pub(crate) async fn builder_for(
        &self,
        network: &str,
    ) -> Result<CosmosBuilder, CosmosConfigError> {
        match (
            CosmosNetwork::from_str(network).ok(),
            self.inner.network.get(network),
        ) {
            (None, None) => Err(CosmosConfigError::UnknownNetwork {
                network: network.to_owned(),
                config: self.path.clone(),
            }),
            (None, Some(config)) => {
                let mut missing = vec![];
                if config.grpc.is_none() {
                    missing.push("grpc");
                }
                if config.chain_id.is_none() {
                    missing.push("chain-id");
                }
                if config.gas_coin.is_none() {
                    missing.push("gas-coin");
                }
                if config.hrp.is_none() {
                    missing.push("hrp");
                }
                match (&config.grpc, &config.chain_id, &config.gas_coin, config.hrp) {
                    (Some(grpc), Some(chain_id), Some(gas_coin), Some(hrp)) => {
                        assert!(missing.is_empty());
                        let mut builder = CosmosBuilder::new(chain_id, gas_coin, hrp, grpc);
                        config.apply_extra_config(&mut builder);
                        Ok(builder)
                    }
                    _ => {
                        assert!(!missing.is_empty());
                        Err(CosmosConfigError::MissingRequiredConfig {
                            missing: missing.join(", "),
                            path: self.path.clone(),
                            network: network.to_owned(),
                        })
                    }
                }
            }
            (Some(network), None) => network
                .builder()
                .await
                .map_err(|source| CosmosConfigError::Builder { source }),
            (Some(_), Some(_)) => todo!(),
        }
    }
}
