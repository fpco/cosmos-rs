use anyhow::Result;
use cosmos::CosmosSecp256k1;

#[derive(clap::Parser)]
pub(crate) struct Opt {
    #[clap(subcommand)]
    sub: Subcommand,
}

#[derive(clap::Parser)]
enum Subcommand {
    /// Generate a Secp256k1 Private key
    GenSecp256k1PrivKey {},
}

pub(crate) async fn go(Opt { sub }: Opt) -> Result<()> {
    match sub {
        Subcommand::GenSecp256k1PrivKey {} => {
            let private_key = CosmosSecp256k1::gen_priv_key();
            let private_key_hex = hex::encode(private_key.secret_bytes()).to_uppercase();
            println!("Private Key: {}", private_key_hex);
        }
    }
    Ok(())
}