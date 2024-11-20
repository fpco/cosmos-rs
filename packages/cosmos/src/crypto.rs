extern crate secp256k1;
extern crate rand;
extern crate base64;

use secp256k1::SecretKey;
use rand::rngs::OsRng;
use rand::RngCore;

/// Represents the secp256k1 crypto algorithm elliptic curve.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Ord, PartialOrd)]
pub struct CosmosSecp256k1;

impl CosmosSecp256k1 {
    /// Generates a private key using secp256k1 elliptic curve.
    pub fn gen_priv_key() -> SecretKey {
        // Create a random number generator
        let mut rng = OsRng::default();
    
        // Generate a random 32-byte array
        let mut secret_key_bytes = [0u8; 32];
        rng.fill_bytes(&mut secret_key_bytes);
    
        // Create the secret key from the random bytes
        let secret_key = SecretKey::from_byte_array(&secret_key_bytes).expect("32 bytes, within curve order");
    
        secret_key
    }
}

#[cfg(test)]
mod tests {
    use secp256k1::{PublicKey, Secp256k1};

    use super::*;

    #[test]
    fn test_gen_priv_key() {
        // Create a new secp256k1 context
        let secp = Secp256k1::new();

        // Generate a new private key
        let private_key = CosmosSecp256k1::gen_priv_key();

        // Generate the corresponding public key
        let public_key = PublicKey::from_secret_key(&secp, &private_key);

        // Convert secret and public keys to hexadecimal
        let private_key_hex = hex::encode(private_key.secret_bytes()).to_uppercase();
        let public_key_hex = hex::encode(public_key.serialize()).to_uppercase();

        assert_eq!(private_key_hex.len(), 64);
        assert_eq!(public_key_hex.len(), 66);
    }
}