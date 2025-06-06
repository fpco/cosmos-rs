#![deny(missing_docs)]
//! Library for communicating with Cosmos blockchains over gRPC
pub use address::{Address, AddressHrp, HasAddress, HasAddressHrp, PublicKeyMethod, RawAddress};
pub use client::{BlockInfo, Cosmos, CosmosTxResponse, HasCosmos};
pub use codeid::{CodeId, ContractType};
#[cfg(feature = "config")]
pub use config::{CosmosConfig, CosmosConfigError};
pub use contract::{Contract, ContractAdmin, HasContract};
pub use cosmos_builder::CosmosBuilder;
pub use cosmos_network::CosmosNetwork;
pub use cosmos_sdk_proto as proto;
pub use cosmos_sdk_proto::cosmos::base::v1beta1::Coin;
pub use error::Error;
pub use ext::TxResponseExt;
pub use gas_multiplier::DynamicGasMultiplier;
pub use parsed_coin::ParsedCoin;
pub use tokenfactory::TokenFactory;
pub use txbuilder::{TxBuilder, TxMessage};
pub use wallet::{SeedPhrase, Wallet};

mod address;
mod authz;
mod client;
mod codeid;
#[cfg(feature = "config")]
mod config;
mod contract;
mod cosmos_builder;
mod cosmos_network;
mod ext;
mod gas_multiplier;
mod injective;
mod parsed_coin;
/// Contains Rujira-specific messages.
pub mod rujira;
mod tokenfactory;
mod txbuilder;
mod wallet;

#[cfg(feature = "clap")]
pub mod clap;

pub mod error;

pub mod gas_price;
pub mod messages;
pub mod osmosis;

/// A result type with our error type provided as the default.
pub type Result<T, E = Error> = std::result::Result<T, E>;
