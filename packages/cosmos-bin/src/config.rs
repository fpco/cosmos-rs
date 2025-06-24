use std::str::FromStr;

use anyhow::Result;
use cosmos::{AddressHrp, ContractType, CosmosConfig, CosmosConfigError};

#[derive(clap::Parser)]
pub(crate) enum Opt {
    /// Print the location of the config file
    File {},
    /// Print the values from the config
    Print {},
    /// Configure a new network
    ///
    /// This forces the caller to provide all required fields.
    /// If you want to make smaller updates, use the set subcommand.
    NewNetwork {
        /// Name to be used for this network
        #[clap(long)]
        name: String,
        /// Primary gRPC endpoint
        #[clap(long)]
        grpc: String,
        /// Chain ID
        #[clap(long)]
        chain_id: String,
        /// Address prefix/HRP
        #[clap(long)]
        hrp: AddressHrp,
        /// Gas coin
        #[clap(long)]
        gas_coin: String,
    },
    /// Set a config value for a specific network
    Set {
        /// Network name
        name: String,
        /// Config key
        key: ConfigKey,
        /// Value
        value: String,
    },
    /// Add a gRPC fallback
    AddFallback {
        /// Network name
        name: String,
        /// gRPC URL
        url: String,
    },
    /// Add a recognized contract code ID
    AddContract {
        /// Network name
        name: String,
        /// Contract type
        contract_type: ContractType,
        /// Code ID
        code_id: u64,
    },
}

// Strum would be more approriate, but serde gives better error messages
#[derive(serde::Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ConfigKey {
    Grpc,
    ChainId,
    Hrp,
    GasCoin,
    LowGasPrice,
    HighGasPrice,
}

impl FromStr for ConfigKey {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(s.to_owned()))
    }
}

fn load(opt: &crate::cli::Opt) -> Result<CosmosConfig, CosmosConfigError> {
    opt.network_opt
        .config
        .as_ref()
        .map_or_else(CosmosConfig::load, |path| {
            CosmosConfig::load_from(path, true)
        })
}

pub(crate) fn go(opt: crate::cli::Opt, inner: Opt) -> Result<()> {
    match inner {
        Opt::File {} => {
            match opt.network_opt.config {
                Some(file) => {
                    tracing::info!(
                        "Config file overridden by command line parameter or environment variable"
                    );
                    println!("{}", file.display());
                }
                None => {
                    println!("{}", CosmosConfig::default_file()?.display())
                }
            }
            Ok(())
        }
        Opt::Print {} => {
            let config = load(&opt)?;
            config.print();
            Ok(())
        }
        Opt::NewNetwork {
            name,
            chain_id,
            hrp,
            grpc,
            gas_coin,
        } => {
            let mut config = load(&opt)?;
            config.new_network(name, grpc, chain_id, gas_coin, hrp);
            config.save()?;
            println!("Changes saved");
            Ok(())
        }
        Opt::Set { name, key, value } => {
            let mut config = load(&opt)?;
            match key {
                ConfigKey::Grpc => config.set_grpc(name, value),
                ConfigKey::ChainId => config.set_chain_id(name, value),
                ConfigKey::Hrp => config.set_hrp(name, value.parse()?),
                ConfigKey::GasCoin => config.set_gas_coin(name, value),
                ConfigKey::LowGasPrice => config.set_low_gas_price(name, value.parse()?),
                ConfigKey::HighGasPrice => config.set_high_gas_price(name, value.parse()?),
            }
            config.save()?;
            println!("Changes saved");
            Ok(())
        }
        Opt::AddFallback { name, url } => {
            let mut config = load(&opt)?;
            config.add_grpc_fallback(name, url);
            config.save()?;
            println!("Changes saved");
            Ok(())
        }
        Opt::AddContract {
            name,
            contract_type,
            code_id,
        } => {
            let mut config = load(&opt)?;
            config.add_contract(name, contract_type, code_id);
            config.save()?;
            println!("Changes saved");
            Ok(())
        }
    }
}
