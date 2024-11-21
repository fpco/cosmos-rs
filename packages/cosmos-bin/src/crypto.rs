use anyhow::Result;
use cosmos::K256;

#[derive(clap::Parser)]
pub(crate) struct Opt {
    #[clap(subcommand)]
    sub: Subcommand,
}

#[derive(clap::Parser)]
enum Subcommand {
    /// Generate a Secp256k1 Private key
    GenK256PrivKey {},
}

pub(crate) async fn go(Opt { sub }: Opt) -> Result<()> {
    match sub {
        Subcommand::GenK256PrivKey {} => {
            let private_key = K256::gen_priv_key();
            let private_key_hex = hex::encode(private_key.secret_bytes()).to_uppercase();
            println!("Private Key: {}", private_key_hex);
        }
    }
    Ok(())
}
