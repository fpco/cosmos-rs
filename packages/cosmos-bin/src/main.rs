mod authz;
mod bank;
mod chain;
mod cli;
mod config;
mod contract;
mod cw3;
mod my_duration;
mod nft;
mod tokenfactory;
mod wallet;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use cli::Subcommand;
use cosmos::AddressHrp;

#[tokio::main]
async fn main() -> Result<()> {
    let cmd = cli::Cmd::parse();
    cmd.opt.init_logger()?;

    tracing::debug!("Verbose logging enabled");

    cmd.subcommand.go(cmd.opt).await
}

impl Subcommand {
    pub(crate) async fn go(self, opt: cli::Opt) -> Result<()> {
        match self {
            Subcommand::Bank { opt: bank_opt } => {
                let cosmos = opt.network_opt.build().await?;
                bank::go(cosmos, bank_opt).await?;
            }
            Subcommand::Wallet { opt } => {
                wallet::go(opt).await?;
            }
            Subcommand::GenerateShellCompletions { shell } => {
                clap_complete::generate(
                    shell,
                    &mut Subcommand::command(),
                    "cosmos",
                    &mut std::io::stdout(),
                );
            }
            Subcommand::Nft {
                opt: inner,
                subcommand,
            } => {
                let cosmos = opt.network_opt.build().await?;
                nft::go(subcommand, inner, cosmos).await?;
            }
            Subcommand::Contract { opt: inner } => {
                let cosmos = opt.network_opt.build().await?;
                contract::go(inner, cosmos).await?;
            }
            Subcommand::Chain { opt: inner } => {
                chain::go(inner, opt).await?;
            }
            Subcommand::TokenFactory { cmd, wallet } => {
                let cosmos = opt.network_opt.build().await?;
                tokenfactory::go(cosmos, wallet, cmd).await?
            }
            Subcommand::Authz { opt: inner } => {
                let cosmos = opt.network_opt.build().await?;
                authz::go(cosmos, inner).await?;
            }
            Subcommand::Cw3 { opt: inner } => {
                let cosmos = opt.network_opt.build().await?;
                cw3::go(cosmos, inner).await?;
            }
            Subcommand::Config { opt: inner } => config::go(opt, inner)?,
        }

        Ok(())
    }
}

fn gen_wallet(hrp: AddressHrp) -> Result<()> {
    let phrase = cosmos::SeedPhrase::random();
    let wallet = phrase.with_hrp(hrp)?;
    let private_key = wallet.get_privkey().private_key.display_secret();
    let public_key = hex::encode(wallet.public_key_bytes());
    println!("Mnemonic: {}", phrase.phrase());
    println!("Address: {wallet}");
    println!("Private Key: {}", private_key);
    println!("Public Key : {}", public_key);
    Ok(())
}
