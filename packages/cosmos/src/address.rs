use std::{
    collections::HashSet,
    fmt::{Debug, Display},
    str::FromStr,
    sync::{Arc, OnceLock},
};

use bech32::{Bech32, Hrp};
use bitcoin::bip32::DerivationPath;
use parking_lot::RwLock;
use serde::de::Visitor;

use crate::{
    error::AddressError, wallet::DerivationPathConfig, Cosmos, CosmosBuilder, CosmosNetwork,
};

/// A raw address value not connected to a specific blockchain.
///
/// This value can be useful for converting addresses between different chains,
/// or for accepting a command line parameter or config value which is
/// chain-agnostic.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Ord, PartialOrd)]
pub struct RawAddress(RawAddressInner);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Ord, PartialOrd)]
enum RawAddressInner {
    Twenty { raw_address: [u8; 20] },
    ThirtyTwo { raw_address: [u8; 32] },
}

impl RawAddress {
    /// Parse a Cosmos-compatible address into an HRP and [RawAddress].
    pub fn parse_with_hrp(s: &str) -> Result<(Hrp, RawAddress), AddressError> {
        let (hrp, data) = bech32::decode(s).map_err(|source| AddressError::InvalidBech32 {
            address: s.to_owned(),
            source,
        })?;

        let data = data.as_slice();
        let raw_address_inner = match data.try_into() {
            Ok(raw_address) => RawAddressInner::Twenty { raw_address },
            Err(_) => data
                .try_into()
                .map(|raw_address| RawAddressInner::ThirtyTwo { raw_address })
                .map_err(|_| AddressError::InvalidByteCount {
                    address: s.to_owned(),
                    actual: data.len(),
                })?,
        };

        let raw_address = RawAddress(raw_address_inner);
        Ok((hrp, raw_address))
    }
}

/// Note that using this instance throws away the Human Readable Parse (HRP) of the address!
impl FromStr for RawAddress {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RawAddress::parse_with_hrp(s).map(|x| x.1)
    }
}

/// Note that using this instance throws away the Human Readable Parse (HRP) of the address!
impl<'de> serde::Deserialize<'de> for RawAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(RawAddressVisitor)
    }
}

struct RawAddressVisitor;

impl Visitor<'_> for RawAddressVisitor {
    type Value = RawAddress;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("RawAddress")
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        RawAddress::parse_with_hrp(s)
            .map(|x| x.1)
            .map_err(E::custom)
    }
}

impl AsRef<[u8]> for RawAddress {
    fn as_ref(&self) -> &[u8] {
        match &self.0 {
            RawAddressInner::Twenty { raw_address } => raw_address,
            RawAddressInner::ThirtyTwo { raw_address } => raw_address,
        }
    }
}

impl From<[u8; 20]> for RawAddress {
    fn from(raw_address: [u8; 20]) -> Self {
        RawAddress(RawAddressInner::Twenty { raw_address })
    }
}

impl From<[u8; 32]> for RawAddress {
    fn from(raw_address: [u8; 32]) -> Self {
        RawAddress(RawAddressInner::ThirtyTwo { raw_address })
    }
}

impl RawAddress {
    /// Generates a new [Address] given the raw address and HRP for the chain.
    pub fn with_hrp(self, hrp: AddressHrp) -> Address {
        Address {
            raw_address: self,
            hrp,
        }
    }
}

/// An address on a Cosmos blockchain.
///
/// This is composed of a [RawAddress] combined with the human-readable part
/// (HRP) for the given chain. HRP is part of the bech32 standard.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address {
    raw_address: RawAddress,
    hrp: AddressHrp,
}

impl Address {
    /// Get the raw bytes without the chain's HRP.
    pub fn raw(self) -> RawAddress {
        self.raw_address
    }

    /// Get the HRP for this address.
    pub fn hrp(self) -> AddressHrp {
        self.hrp
    }
}

/// The method used for hashing public keys into a byte representation.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PublicKeyMethod {
    /// Cosmos standard is to use a combination of SHA2 256 and ripemd160.
    Cosmos,
    /// Ethereum, and some Cosmos chains like Injective, use keccak3.
    Ethereum,
}

impl Display for Address {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let raw_address = match self.raw_address.0 {
            RawAddressInner::Twenty { raw_address } => raw_address.to_vec(),
            RawAddressInner::ThirtyTwo { raw_address } => raw_address.to_vec(),
        };
        let hrp = Hrp::parse(self.hrp.0).expect("Invalid HRP");
        bech32::encode_to_fmt::<Bech32, _>(fmt, hrp, &raw_address[..]).expect("Encode issue");
        Ok(())
    }
}

impl From<Address> for String {
    fn from(address: Address) -> Self {
        address.to_string()
    }
}

impl From<&Address> for String {
    fn from(address: &Address) -> Self {
        address.to_string()
    }
}

impl FromStr for Address {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RawAddress::parse_with_hrp(s).map(|(hrp, raw_address)| {
            raw_address.with_hrp(
                AddressHrp::from_hrp(hrp).expect("parse_with_hrp gave back in invalid HRP"),
            )
        })
    }
}

/// Anything which has an on-chain [Address].
pub trait HasAddress: HasAddressHrp {
    /// Get the raw address itself.
    fn get_address(&self) -> Address;

    /// Get the string representation of the address.
    fn get_address_string(&self) -> String {
        self.get_address().to_string()
    }
}

impl HasAddress for Address {
    fn get_address(&self) -> Address {
        *self
    }
}

impl<T: HasAddress> HasAddress for &T {
    fn get_address(&self) -> Address {
        HasAddress::get_address(*self)
    }
}

/// The human-readable part (HRP) of a Cosmos address.
///
/// HRPs are part of the bech32 standard. All addresses encoded with
/// bech32--which includes all Cosmos chains--have a human-readable part, such
/// as `cosmos` for CosmosHub or `osmo` for Osmosis.  This trait allows us to
/// query the HRP for various types within this package that know their HRP.
///
/// This library internally shares multiple copies of the same HRP for both
/// efficiency and ease of use of this library: it allows both this data type,
/// as well as [Address], to be [Copy].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, serde::Serialize)]
pub struct AddressHrp(&'static str);

impl FromStr for AddressHrp {
    type Err = AddressError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        AddressHrp::new(s)
    }
}

impl<'de> serde::Deserialize<'de> for AddressHrp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(AddressHrpVisitor)
    }
}

struct AddressHrpVisitor;

impl Visitor<'_> for AddressHrpVisitor {
    type Value = AddressHrp;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("AddressHrp")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse::<AddressHrp>()
            .map_err(|e| E::custom(e.to_string()))
    }
}

impl AddressHrp {
    /// The default [DerivationPath] for this HRP.
    ///
    /// Some chains follow Ethereum rules, notably Injective. For all other
    /// chains we default to Cosmos defaults.
    pub fn default_derivation_path(self) -> Arc<DerivationPath> {
        self.default_derivation_path_with_index(0)
    }

    /// Same as [Self::default_derivation_path], but includes an index.
    pub fn default_derivation_path_with_index(self, index: u64) -> Arc<DerivationPath> {
        match self.as_str() {
            "inj" => DerivationPathConfig::ethereum_numbered(index).as_derivation_path(),
            _ => DerivationPathConfig::cosmos_numbered(index).as_derivation_path(),
        }
    }

    /// The default public key method for this HRP.
    ///
    /// Public keys are hashed into bytes used for wallet addresses. This
    /// represents the strategy used. Some chains, notably Injective, use
    /// Ethereum's method. The default is to use Cosmos's method.
    pub fn default_public_key_method(self) -> PublicKeyMethod {
        match self.as_str() {
            "inj" => PublicKeyMethod::Ethereum,
            _ => PublicKeyMethod::Cosmos,
        }
    }
}

impl Display for AddressHrp {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

type AddressHrpSet = RwLock<HashSet<&'static str>>;
static ADDRESS_HRPS: OnceLock<AddressHrpSet> = OnceLock::new();
impl AddressHrp {
    fn get_set() -> &'static AddressHrpSet {
        ADDRESS_HRPS.get_or_init(|| RwLock::new(HashSet::new()))
    }

    /// Generate a new value from a [String]-like value.
    pub fn new(s: impl AsRef<str>) -> Result<Self, AddressError> {
        let s = s.as_ref();
        if !is_valid_hrp(s) {
            return Err(AddressError::InvalidHrp { hrp: s.to_owned() });
        }
        let set = Self::get_set();
        {
            if let Some(s) = set.read().get(s) {
                return Ok(AddressHrp(s));
            }
        }
        let mut guard = set.write();
        // Deal with race condition: was this added between our read and now?
        if let Some(s) = guard.get(s) {
            return Ok(AddressHrp(s));
        }
        let s = Box::leak(s.to_owned().into_boxed_str());
        guard.insert(s);
        Ok(AddressHrp(s))
    }

    /// Minor optimization over [AddressHrp::new]: use a static string for initializing.
    ///
    /// Note that this bypasses the check that the HRP is valid.
    pub fn from_static(s: &'static str) -> Self {
        let set = Self::get_set();
        {
            if let Some(s) = set.read().get(s) {
                return AddressHrp(s);
            }
        }
        let mut guard = set.write();
        // Deal with race condition: was this added between our read and now?
        if let Some(s) = guard.get(s) {
            return AddressHrp(s);
        }
        guard.insert(s);
        AddressHrp(s)
    }

    /// Minor optimization over [AddressHrp::new]: use an owned [String] for initializing.
    pub fn from_string(s: String) -> Result<Self, AddressError> {
        if !is_valid_hrp(&s) {
            return Err(AddressError::InvalidHrp { hrp: s });
        }
        let set = Self::get_set();
        {
            if let Some(s) = set.read().get(&*s) {
                return Ok(AddressHrp(s));
            }
        }
        let mut guard = set.write();
        // Deal with race condition: was this added between our read and now?
        if let Some(s) = guard.get(&*s) {
            return Ok(AddressHrp(s));
        }
        let s = Box::leak(s.into_boxed_str());
        guard.insert(s);
        Ok(AddressHrp(s))
    }

    /// Minor optimization over [AddressHrp::from_string]: use an owned [Hrp] for initializing.
    pub fn from_hrp(s: Hrp) -> Result<Self, AddressError> {
        let s = s.to_lowercase();
        let set = Self::get_set();
        {
            if let Some(s) = set.read().get(&*s) {
                return Ok(AddressHrp(s));
            }
        }
        let mut guard = set.write();
        // Deal with race condition: was this added between our read and now?
        if let Some(s) = guard.get(&*s) {
            return Ok(AddressHrp(s));
        }
        let s = Box::leak(s.into_boxed_str());
        guard.insert(s);
        Ok(AddressHrp(s))
    }

    /// Get the raw string HRP
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

fn is_valid_hrp(hrp: &str) -> bool {
    // Unfortunately `check_hrp` isn't exposed from bech32, so doing something silly...
    Hrp::parse(hrp).is_ok()
}

/// Trait for any values that can report their bech32 HRP (human-readable part).
///
pub trait HasAddressHrp {
    /// Return the HRP
    fn get_address_hrp(&self) -> AddressHrp;
}

impl<T: HasAddressHrp> HasAddressHrp for &T {
    fn get_address_hrp(&self) -> AddressHrp {
        (*self).get_address_hrp()
    }
}

impl HasAddressHrp for Address {
    fn get_address_hrp(&self) -> AddressHrp {
        self.hrp
    }
}

impl HasAddressHrp for Cosmos {
    fn get_address_hrp(&self) -> AddressHrp {
        self.get_cosmos_builder().get_address_hrp()
    }
}

impl HasAddressHrp for CosmosBuilder {
    fn get_address_hrp(&self) -> AddressHrp {
        self.hrp()
    }
}

impl HasAddressHrp for CosmosNetwork {
    fn get_address_hrp(&self) -> AddressHrp {
        AddressHrp::from_static(match self {
            CosmosNetwork::JunoTestnet | CosmosNetwork::JunoMainnet | CosmosNetwork::JunoLocal => {
                "juno"
            }
            CosmosNetwork::OsmosisMainnet
            | CosmosNetwork::OsmosisTestnet
            | CosmosNetwork::OsmosisLocal => "osmo",
            CosmosNetwork::WasmdLocal => "wasm",
            CosmosNetwork::SeiMainnet | CosmosNetwork::SeiTestnet => "sei",
            CosmosNetwork::StargazeTestnet | CosmosNetwork::StargazeMainnet => "stars",
            CosmosNetwork::InjectiveTestnet | CosmosNetwork::InjectiveMainnet => "inj",
            CosmosNetwork::NeutronMainnet | CosmosNetwork::NeutronTestnet => "neutron",
        })
    }
}

impl serde::Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(AddressVisitor)
    }
}

struct AddressVisitor;

impl Visitor<'_> for AddressVisitor {
    type Value = Address;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("Cosmos address")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse().map_err(|e| E::custom(e))
    }
}

impl Debug for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\"{self}\"")
    }
}

#[cfg(test)]
mod tests {
    use quickcheck::Arbitrary;

    use super::*;

    impl Arbitrary for AddressHrp {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            AddressHrp::from_static(
                g.choose(&["juno", "stars", "osmo", "wasm", "inj", "cosmos"])
                    .unwrap(),
            )
        }
    }

    impl Arbitrary for RawAddress {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            if bool::arbitrary(g) {
                let mut raw_address = [0; 20];
                for byte in &mut raw_address {
                    *byte = u8::arbitrary(g);
                }
                RawAddress(RawAddressInner::Twenty { raw_address })
            } else {
                let mut raw_address = [0; 32];
                for byte in &mut raw_address {
                    *byte = u8::arbitrary(g);
                }
                RawAddress(RawAddressInner::ThirtyTwo { raw_address })
            }
        }
    }

    quickcheck::quickcheck! {
        fn roundtrip_address(hrp: AddressHrp, raw_address: RawAddress) -> bool {
            let address1 = raw_address.with_hrp(hrp);
            let s1 = address1.to_string();
            let address2: Address = s1.parse().unwrap();
            let s2 = address2.to_string();
            assert_eq!(s1, s2);
            assert_eq!(address1, address2);
            true
        }
    }

    #[test]
    fn spot_roundtrip_osmo() {
        const S: &str = "osmo168gdk6r58jdwfv49kuesq2rs747jawnn4ryvyk";
        let address: Address = S.parse().unwrap();
        assert_eq!(S, &address.to_string());
    }

    #[test]
    fn spot_roundtrip_juno() {
        const S: &str = "juno168gdk6r58jdwfv49kuesq2rs747jawnnt2584c";
        let address: Address = S.parse().unwrap();
        assert_eq!(S, &address.to_string());
    }

    #[test]
    fn valid_hrp() {
        AddressHrp::new("juno").unwrap();
        AddressHrp::new("osmo").unwrap();
        AddressHrp::new("btc").unwrap();
        AddressHrp::new("foobar").unwrap();

        // To my surprise this is actually allowed per spec
        AddressHrp::new("osmo1").unwrap();
        AddressHrp::new("foobar2").unwrap();
    }

    #[test]
    fn invalid_hrp() {
        AddressHrp::new("juno with space").unwrap_err();
    }
}
