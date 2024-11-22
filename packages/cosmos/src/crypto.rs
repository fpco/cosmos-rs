extern crate base64;
extern crate rand;

use bitcoin::key::Secp256k1;
use bitcoin::secp256k1::{PublicKey, SecretKey};
use rand::rngs::OsRng;
use rand::RngCore;

/// Represents the secp256k1 crypto algorithm elliptic curve.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyPair {
    pub private_key: SecretKey,
    pub public_key: PublicKey,
}

/// Represents the secp256k1 crypto algorithm elliptic curve.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Ord, PartialOrd)]
pub struct K256;

impl K256 {
    /// Generates a private key using secp256k1 elliptic curve.
    pub fn gen_priv_key() -> SecretKey {
        let mut rng = OsRng;
        let mut secret_key_bytes = [0u8; 32];
        rng.fill_bytes(&mut secret_key_bytes);
        SecretKey::from_slice(&secret_key_bytes).expect("32 bytes, within curve order")
    }

    /// Generates a private/public key pair using secp256k1 elliptic curve.
    pub fn gen_key_pair() -> KeyPair {
        let private_key = Self::gen_priv_key();
        let secp = Secp256k1::new();
        let public_key = PublicKey::from_secret_key(&secp, &private_key);
        KeyPair {
            private_key,
            public_key,
        }
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{PublicKey, Secp256k1};

    use super::*;

    #[test]
    fn test_gen_priv_key() {
        // Create a new secp256k1 context
        let secp = Secp256k1::new();

        // Generate a new private key
        let private_key = K256::gen_priv_key();

        // Generate the corresponding public key
        let public_key = PublicKey::from_secret_key(&secp, &private_key);

        // Convert secret and public keys to hexadecimal
        let private_key_hex = hex::encode(private_key.secret_bytes()).to_uppercase();
        let public_key_hex = hex::encode(public_key.serialize()).to_uppercase();

        assert_eq!(private_key_hex.len(), 64);
        assert_eq!(public_key_hex.len(), 66);
    }

    #[test]
    fn test_gen_key_pair() {
        // Generate a new private key
        let key_pair = K256::gen_key_pair();

        // Convert secret and public keys to hexadecimal
        let private_key_hex = hex::encode(key_pair.private_key.secret_bytes()).to_uppercase();
        let public_key_hex = hex::encode(key_pair.public_key.serialize()).to_uppercase();

        assert_eq!(private_key_hex.len(), 64);
        assert_eq!(public_key_hex.len(), 66);
    }
}
