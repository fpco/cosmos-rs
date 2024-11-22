use anyhow::Result;
use cosmos::K256;

#[derive(clap::Parser)]
pub(crate) struct Opt {
    #[clap(subcommand)]
    sub: Subcommand,
}

#[derive(clap::Parser)]
enum Subcommand {
    /// Generate a Secp256k1 Private/Public key pair
    GenK256KeyPair {},
}

pub(crate) async fn go(Opt { sub }: Opt) -> Result<()> {
    match sub {
        Subcommand::GenK256KeyPair {} => {
            let key_pair = K256::gen_key_pair();
            let private_key_hex = hex::encode(key_pair.private_key.secret_bytes()).to_uppercase();
            let public_key_hex = hex::encode(key_pair.public_key.serialize()).to_uppercase();
            println!("Private Key: {}", private_key_hex);
            println!("Public Key : {}", public_key_hex);
        }
    }
    Ok(())
}
