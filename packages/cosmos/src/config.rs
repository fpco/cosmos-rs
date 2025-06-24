use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    str::FromStr,
};

use figment::{
    providers::{Env, Format, Toml},
    Figment,
};

use crate::{gas_price::GasPriceMethod, AddressHrp, ContractType, CosmosBuilder, CosmosNetwork};

/// Configuration overrides for individual network
#[derive(Debug)]
pub struct CosmosConfig {
    path: PathBuf,
    inner: CosmosConfigInner,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct CosmosConfigInner {
    #[serde(default)]
    network: HashMap<String, NetworkConfig>,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Default)]
#[serde(rename_all = "kebab-case")]
struct NetworkConfig {
    grpc: Option<String>,
    chain_id: Option<String>,
    gas_coin: Option<String>,
    hrp: Option<AddressHrp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    grpc_fallbacks: Vec<String>,
    gas_multiplier: Option<f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    code_ids: BTreeMap<ContractType, u64>,
    low_gas_price: Option<f64>,
    high_gas_price: Option<f64>,
}

impl NetworkConfig {
    fn apply_base_config(&self, builder: &mut CosmosBuilder) {
        if let Some(grpc) = &self.grpc {
            builder.set_grpc_url(grpc);
        }
        if let Some(chain_id) = self.chain_id.clone() {
            builder.set_chain_id(chain_id);
        }
        if let Some(gas_coin) = self.gas_coin.clone() {
            builder.set_gas_coin(gas_coin);
        }
        if let Some(hrp) = self.hrp {
            builder.set_hrp(hrp);
        }
    }
    fn apply_extra_config(&self, builder: &mut CosmosBuilder) {
        for fallback in &self.grpc_fallbacks {
            builder.add_grpc_fallback_url(fallback);
        }
        for (contract_type, code_id) in &self.code_ids {
            builder.set_code_id(*contract_type, *code_id);
        }
        if let Some(gas_multiplier) = self.gas_multiplier {
            builder.set_gas_estimate_multiplier(gas_multiplier)
        }
        let gas_price = match (self.low_gas_price, self.high_gas_price) {
            (None, None) => None,
            (Some(x), None) => Some((x, x)),
            (None, Some(x)) => Some((x, x)),
            (Some(x), Some(y)) => Some((x, y)),
        };
        if let Some((low, high)) = gas_price {
            builder.set_gas_price_method(GasPriceMethod::new_static(low, high));
        }
        for (contract_type, code_id) in &self.code_ids {
            builder.set_code_id(*contract_type, *code_id);
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
    #[error(transparent)]
    TomlSerialization { source: toml::ser::Error },
    #[error("Unable to write config to {}: {source}", path.display())]
    ConfigWrite {
        source: std::io::Error,
        path: PathBuf,
    },
}

impl CosmosConfig {
    /// Find the default config file location
    pub fn default_file() -> Result<PathBuf, CosmosConfigError> {
        let dirs = directories::ProjectDirs::from("com", "fpco", "cosmos-rs")
            .ok_or(CosmosConfigError::ProjectDirsNotFound)?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Load the config values from the default config file location
    pub fn load() -> Result<CosmosConfig, CosmosConfigError> {
        Self::load_from(&Self::default_file()?, false)
    }

    /// Load the config values from the specified file
    pub fn load_from(config: &Path, required: bool) -> Result<CosmosConfig, CosmosConfigError> {
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
            (Some(network), Some(config)) => {
                let mut builder = network
                    .builder()
                    .await
                    .map_err(|source| CosmosConfigError::Builder { source })?;
                config.apply_base_config(&mut builder);
                config.apply_extra_config(&mut builder);
                Ok(builder)
            }
        }
    }

    /// Print out a description of the config file
    pub fn print(&self) {
        println!("Location: {}", self.path.display());
        let mut networks = self.inner.network.iter().collect::<Vec<_>>();
        networks.sort_by_key(|x| x.0);
        for (
            network,
            NetworkConfig {
                grpc,
                chain_id,
                gas_coin,
                hrp,
                grpc_fallbacks,
                gas_multiplier,
                code_ids,
                low_gas_price,
                high_gas_price,
            },
        ) in networks
        {
            println!();
            println!("{network}");
            if let Some(grpc) = grpc {
                println!("Primary endpoint: {grpc}");
            }
            for (idx, fallback) in grpc_fallbacks.iter().enumerate() {
                println!("Fallback #{}: {fallback}", idx + 1);
            }
            if let Some(chain_id) = chain_id {
                println!("Chain ID: {chain_id}");
            }
            if let Some(gas_coin) = gas_coin {
                println!("Gas coin: {gas_coin}");
            }
            if let Some(hrp) = hrp {
                println!("Address prefix (HRP): {hrp}");
            }
            if let Some(gas_multiplier) = gas_multiplier {
                println!("Gas multiplier: {gas_multiplier}");
            }
            if let Some(low) = low_gas_price {
                println!("Low gas price: {low}");
            }
            if let Some(high) = high_gas_price {
                println!("High gas price: {high}");
            }
            for (contract_type, code_id) in code_ids {
                println!("Code ID for {contract_type}: {code_id}");
            }
        }
    }

    /// Add a new network to the config
    pub fn new_network(
        &mut self,
        name: String,
        grpc: String,
        chain_id: String,
        gas_coin: String,
        hrp: AddressHrp,
    ) {
        self.inner.network.insert(
            name,
            NetworkConfig {
                grpc: Some(grpc),
                chain_id: Some(chain_id),
                gas_coin: Some(gas_coin),
                hrp: Some(hrp),
                grpc_fallbacks: vec![],
                gas_multiplier: None,
                code_ids: BTreeMap::new(),
                low_gas_price: None,
                high_gas_price: None,
            },
        );
    }

    /// Write the config to the original file.
    pub fn save(&self) -> Result<(), CosmosConfigError> {
        self.save_to(&self.path)
    }

    /// Write the config to the given file.
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<(), CosmosConfigError> {
        let s = toml::to_string_pretty(&self.inner)
            .map_err(|source| CosmosConfigError::TomlSerialization { source })?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs_err::create_dir_all(parent).map_err(|source| CosmosConfigError::ConfigWrite {
                source,
                path: path.to_owned(),
            })?;
        }
        fs_err::write(path, s).map_err(|source| CosmosConfigError::ConfigWrite {
            source,
            path: path.to_owned(),
        })
    }

    /// Set the primary gRPC endpoint
    pub fn set_grpc(&mut self, name: String, url: String) {
        self.inner.network.entry(name).or_default().grpc = Some(url);
    }

    /// Set the chain ID
    pub fn set_chain_id(&mut self, name: String, chain_id: String) {
        self.inner.network.entry(name).or_default().chain_id = Some(chain_id);
    }

    /// Set the Human Readable Part (HRP)
    pub fn set_hrp(&mut self, name: String, hrp: AddressHrp) {
        self.inner.network.entry(name).or_default().hrp = Some(hrp);
    }

    /// Set the gas coin
    pub fn set_gas_coin(&mut self, name: String, gas_coin: String) {
        self.inner.network.entry(name).or_default().gas_coin = Some(gas_coin);
    }

    /// Add a gRPC fallback
    pub fn add_grpc_fallback(&mut self, name: String, url: String) {
        self.inner
            .network
            .entry(name)
            .or_default()
            .grpc_fallbacks
            .push(url);
    }

    /// Add a new contract type/code ID mapping.
    pub fn add_contract(&mut self, name: String, contract_type: ContractType, code_id: u64) {
        self.inner
            .network
            .entry(name)
            .or_default()
            .code_ids
            .insert(contract_type, code_id);
    }

    /// Set the low gas price
    pub fn set_low_gas_price(&mut self, name: String, low: f64) {
        self.inner.network.entry(name).or_default().low_gas_price = Some(low);
    }

    /// Set the high gas price
    pub fn set_high_gas_price(&mut self, name: String, high: f64) {
        self.inner.network.entry(name).or_default().high_gas_price = Some(high);
    }
}

impl CosmosNetwork {
    /// Generating a builder, respecting the default config file.
    pub async fn builder_with_config(&self) -> Result<CosmosBuilder, CosmosConfigError> {
        let config = CosmosConfig::load()?;
        config.builder_for(self.as_str()).await
    }
}
