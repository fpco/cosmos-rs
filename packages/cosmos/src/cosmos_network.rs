use std::{collections::HashMap, str::FromStr};

use serde::de::Visitor;
use strum_macros::{EnumString, IntoStaticStr};

use crate::{error::BuilderError, gas_price::GasPriceMethod, Cosmos, CosmosBuilder, HasAddressHrp};

/// A set of known networks.
///
/// This library is designed to work with arbitrary other chains too, but
/// providing this built-in list is intended to provide convenience for users of
/// the library.
///
/// Generally you'll want to use either [CosmosNetwork::builder] or [CosmosNetwork::connect].
#[derive(
    Clone,
    Copy,
    Debug,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    EnumString,
    IntoStaticStr,
    strum_macros::Display,
)]
#[strum(serialize_all = "kebab-case")]
#[allow(missing_docs)]
pub enum CosmosNetwork {
    JunoTestnet,
    JunoMainnet,
    JunoLocal,
    OsmosisMainnet,
    OsmosisTestnet,
    OsmosisLocal,
    WasmdLocal,
    SeiMainnet,
    SeiTestnet,
    StargazeTestnet,
    StargazeMainnet,
    InjectiveTestnet,
    InjectiveMainnet,
    NeutronMainnet,
    NeutronTestnet,
}

impl CosmosNetwork {
    /// Returns ['true'] if the network is mainnet
    pub fn is_mainnet(&self) -> bool {
        match self {
            CosmosNetwork::JunoTestnet
            | CosmosNetwork::JunoLocal
            | CosmosNetwork::OsmosisLocal
            | CosmosNetwork::WasmdLocal
            | CosmosNetwork::OsmosisTestnet
            | CosmosNetwork::SeiTestnet
            | CosmosNetwork::StargazeTestnet
            | CosmosNetwork::InjectiveTestnet
            | CosmosNetwork::NeutronTestnet => false,
            CosmosNetwork::JunoMainnet
            | CosmosNetwork::OsmosisMainnet
            | CosmosNetwork::SeiMainnet
            | CosmosNetwork::StargazeMainnet
            | CosmosNetwork::InjectiveMainnet
            | CosmosNetwork::NeutronMainnet => true,
        }
    }

    /// Convenience method to make a [Self::builder] and then [CosmosBuilder::build] it.
    pub async fn connect(self) -> Result<Cosmos, BuilderError> {
        self.builder().await?.build()
    }

    /// Construct a [CosmosBuilder] for this network with default values.
    ///
    /// Combines [Self::builder_local] and [Self::load_settings].
    ///
    /// If you have an existing [reqwest::Client], consider using [Self::builder_with].
    pub async fn builder(self) -> Result<CosmosBuilder, BuilderError> {
        self.builder_with(&reqwest::Client::new()).await
    }

    /// Same as [Self::builder] but takes an existing [reqwest::Client]
    pub async fn builder_with(
        self,
        client: &reqwest::Client,
    ) -> Result<CosmosBuilder, BuilderError> {
        let mut builder = self.builder_local();
        self.load_settings(client, &mut builder).await?;
        Ok(builder)
    }

    /// Construct a [CosmosBuilder] without loading settings from the internet.
    pub fn builder_local(self) -> CosmosBuilder {
        let mut builder = CosmosBuilder::new(
            self.chain_id(),
            self.gas_coin(),
            self.get_address_hrp(),
            self.grpc_url(),
        );
        self.local_settings(&mut builder);
        builder
    }

    /// Chain ID for the network
    pub fn chain_id(self) -> &'static str {
        match self {
            CosmosNetwork::JunoTestnet => "uni-6",
            CosmosNetwork::JunoMainnet => "juno-1",
            CosmosNetwork::JunoLocal => "testing",
            CosmosNetwork::OsmosisMainnet => "osmosis-1",
            CosmosNetwork::OsmosisTestnet => "osmo-test-5",
            CosmosNetwork::OsmosisLocal => "localosmosis",
            CosmosNetwork::WasmdLocal => "localwasmd",
            CosmosNetwork::SeiMainnet => "pacific-1",
            CosmosNetwork::SeiTestnet => "atlantic-2",
            CosmosNetwork::StargazeTestnet => "elgafar-1",
            CosmosNetwork::StargazeMainnet => "stargaze-1",
            CosmosNetwork::InjectiveTestnet => "injective-888",
            CosmosNetwork::InjectiveMainnet => "injective-1",
            CosmosNetwork::NeutronMainnet => "neutron-1",
            CosmosNetwork::NeutronTestnet => "pion-1",
        }
    }

    /// Gas coin for the network
    pub fn gas_coin(self) -> &'static str {
        match self {
            CosmosNetwork::JunoTestnet | CosmosNetwork::JunoLocal => "ujunox",
            CosmosNetwork::JunoMainnet => "ujuno",
            CosmosNetwork::OsmosisMainnet
            | CosmosNetwork::OsmosisTestnet
            | CosmosNetwork::OsmosisLocal => "uosmo",
            CosmosNetwork::WasmdLocal => "uwasm",
            CosmosNetwork::SeiMainnet | CosmosNetwork::SeiTestnet => "usei",
            CosmosNetwork::StargazeTestnet | CosmosNetwork::StargazeMainnet => "ustars",
            CosmosNetwork::InjectiveTestnet | CosmosNetwork::InjectiveMainnet => "inj",
            CosmosNetwork::NeutronMainnet | CosmosNetwork::NeutronTestnet => "untrn",
        }
    }

    /// Default gRPC URL for the network
    pub fn grpc_url(self) -> &'static str {
        match self {
            CosmosNetwork::JunoTestnet => "http://juno-testnet-grpc.polkachu.com:12690",
            // Found at: https://cosmos.directory/juno/nodes
            CosmosNetwork::JunoMainnet => "http://juno-grpc.polkachu.com:12690",
            CosmosNetwork::JunoLocal => "http://localhost:9090",
            // Found at: https://docs.osmosis.zone/networks/
            CosmosNetwork::OsmosisMainnet => "https://grpc.osmosis.zone",
            // Others available at: https://docs.osmosis.zone/networks/
            CosmosNetwork::OsmosisTestnet => "https://grpc.osmotest5.osmosis.zone",
            CosmosNetwork::OsmosisLocal => "http://localhost:9090",
            CosmosNetwork::WasmdLocal => "http://localhost:9090",
            CosmosNetwork::SeiMainnet => "https://grpc.sei-apis.com",
            CosmosNetwork::SeiTestnet => "https://grpc-testnet.sei-apis.com",
            // https://github.com/cosmos/chain-registry/blob/master/testnets/stargazetestnet/chain.json
            CosmosNetwork::StargazeTestnet => "http://grpc-1.elgafar-1.stargaze-apis.com:26660",
            // https://github.com/cosmos/chain-registry/blob/master/stargaze/chain.json
            CosmosNetwork::StargazeMainnet => "http://stargaze-grpc.polkachu.com:13790",
            // https://docs.injective.network/develop/public-endpoints/
            CosmosNetwork::InjectiveTestnet => {
                "https://testnet.sentry.chain.grpc.injective.network"
            }
            // https://docs.injective.network/develop/public-endpoints/
            CosmosNetwork::InjectiveMainnet => "https://sentry.chain.grpc.injective.network",
            CosmosNetwork::NeutronMainnet => "http://grpc-kralum.neutron-1.neutron.org",
            CosmosNetwork::NeutronTestnet => "http://grpc-falcron.pion-1.ntrn.tech",
        }
    }

    /// Override other settings based on chain.
    pub fn local_settings(self, builder: &mut CosmosBuilder) {
        match self {
            CosmosNetwork::JunoTestnet
            | CosmosNetwork::JunoMainnet
            | CosmosNetwork::OsmosisTestnet
            | CosmosNetwork::StargazeTestnet
            | CosmosNetwork::StargazeMainnet
            | CosmosNetwork::NeutronMainnet
            | CosmosNetwork::NeutronTestnet => (),
            CosmosNetwork::OsmosisMainnet => {
                builder.set_osmosis_mainnet_chain_paused();
                // We need a very wide band on Osmosis gas prices due to bugs in
                // the EIP fee market mechanism. Do lots of smaller attempts to
                // avoid overpaying by too much.
                builder.set_gas_price_retry_attempts(Some(12));
            }
            CosmosNetwork::SeiMainnet => {
                // https://github.com/sei-protocol/chain-registry/blob/main/gas.json
                builder.set_gas_price(0.02, 0.2);
                builder.set_gas_price_retry_attempts(Some(6));
            }
            CosmosNetwork::SeiTestnet => {
                // https://github.com/sei-protocol/chain-registry/blob/main/gas.json
                builder.set_gas_price(0.08, 0.8);
                builder.set_gas_price_retry_attempts(Some(6));
            }
            CosmosNetwork::JunoLocal | CosmosNetwork::WasmdLocal | CosmosNetwork::OsmosisLocal => {
                // fail faster during testing
                builder.set_transaction_attempts(Some(3));
            }
            CosmosNetwork::InjectiveTestnet => {
                // https://github.com/cosmos/chain-registry/blob/master/testnets/injectivetestnet/chain.json
                builder.set_gas_price(500000000.0, 900000000.0);
            }
            CosmosNetwork::InjectiveMainnet => {
                // https://github.com/cosmos/chain-registry/blob/master/injective/chain.json
                builder.set_gas_price(500000000.0, 900000000.0);
            }
        }
    }

    /// Load settings, like gas fees, from the internet.
    pub async fn load_settings(
        self,
        client: &reqwest::Client,
        builder: &mut CosmosBuilder,
    ) -> Result<(), BuilderError> {
        match self {
            CosmosNetwork::JunoTestnet
            | CosmosNetwork::JunoMainnet
            | CosmosNetwork::JunoLocal
            | CosmosNetwork::OsmosisTestnet
            | CosmosNetwork::OsmosisLocal
            | CosmosNetwork::WasmdLocal
            | CosmosNetwork::StargazeTestnet
            | CosmosNetwork::StargazeMainnet
            | CosmosNetwork::InjectiveTestnet
            | CosmosNetwork::InjectiveMainnet
            | CosmosNetwork::NeutronMainnet
            | CosmosNetwork::NeutronTestnet => Ok(()),
            CosmosNetwork::OsmosisMainnet => {
                builder.set_gas_price_method(
                    GasPriceMethod::new_osmosis_mainnet(builder.get_osmosis_gas_params()).await?,
                );
                Ok(())
            }
            CosmosNetwork::SeiMainnet => {
                let min_gas_price = get_sei_min_gas_price(client, "pacific-1").await?;

                builder.set_gas_price(min_gas_price, min_gas_price * 10.0);
                Ok(())
            }
            CosmosNetwork::SeiTestnet => {
                let min_gas_price = get_sei_min_gas_price(client, "atlantic-2").await?;

                builder.set_gas_price(min_gas_price, min_gas_price * 10.0);
                Ok(())
            }
        }
    }

    /// Returns the string represenation for that network.
    #[inline]
    pub fn as_str(&self) -> &'static str {
        self.into()
    }
}

async fn get_sei_min_gas_price(
    client: &reqwest::Client,
    chain_id: &str,
) -> Result<f64, BuilderError> {
    #[derive(serde::Deserialize, Debug)]
    struct SeiGasConfigItem {
        pub min_gas_price: f64,
    }

    const URL: &str = "https://raw.githubusercontent.com/sei-protocol/chain-registry/main/gas.json";

    let configs = load_json::<HashMap<String, SeiGasConfigItem>>(URL, client).await?;

    configs
        .get(chain_id)
        .map(|config| config.min_gas_price)
        .ok_or_else(|| BuilderError::SeiGasConfigNotFound {
            chain_id: chain_id.to_owned(),
            url: URL.to_owned(),
        })
}

async fn load_json<T>(url: &str, client: &reqwest::Client) -> Result<T, BuilderError>
where
    T: serde::de::DeserializeOwned,
{
    async {
        client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
    .await
    .map_err(|source| BuilderError::DownloadChainInfo {
        url: url.to_owned(),
        source,
    })
}

impl serde::Serialize for CosmosNetwork {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for CosmosNetwork {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(CosmosNetworkVisitor)
    }
}

struct CosmosNetworkVisitor;

impl<'de> Visitor<'de> for CosmosNetworkVisitor {
    type Value = CosmosNetwork;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("CosmosNetwork")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        CosmosNetwork::from_str(v).map_err(E::custom)
    }
}
