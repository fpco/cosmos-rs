mod authz;
mod chain;
mod cli;
mod contract;
mod cw3;
mod my_duration;
mod nft;
mod parsed_coin;
mod tokenfactory;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use cli::Subcommand;
use cosmos::{
    proto::{
        cosmos::{bank::v1beta1::MsgSend, base::abci::v1beta1::TxResponse},
        traits::Message,
    },
    AddressHrp, BlockInfo, Coin, HasAddress, HasAddressHrp, TxBuilder,
};

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
            Subcommand::ShowConfig {} => {
                let cosmos = opt.network_opt.into_builder().await?;
                println!("{:#?}", cosmos);
            }
            Subcommand::PrintBalances { address, height } => {
                let cosmos = opt.network_opt.build().await?;
                let balances = cosmos.at_height(height).all_balances(address).await?;
                for Coin { denom, amount } in &balances {
                    println!("{amount}{denom}");
                }
                if balances.is_empty() {
                    println!("0");
                }
            }
            Subcommand::GenWallet { address_type } => gen_wallet(address_type)?,
            Subcommand::PrintAddress { hrp, phrase } => {
                println!("{}", phrase.with_hrp(hrp)?);
            }
            Subcommand::SendCoins {
                tx_opt,
                dest,
                coins,
            } => {
                let cosmos = opt.network_opt.build().await?;
                let address_type = cosmos.get_address_hrp();
                let wallet = tx_opt.get_wallet(address_type)?;
                let mut builder = TxBuilder::default();
                builder.add_message(MsgSend {
                    from_address: wallet.get_address_string(),
                    to_address: dest.get_address_string(),
                    amount: coins.into_iter().map(|x| x.into()).collect(),
                });
                builder.set_optional_memo(tx_opt.memo);
                let txres = builder.sign_and_broadcast(&cosmos, &wallet).await?;

                println!("{}", txres.txhash);
            }
            Subcommand::ShowTx {
                txhash,
                complete,
                pretty,
            } => {
                let cosmos = opt.network_opt.build().await?;
                let TxResponse {
                    height,
                    txhash: _,
                    codespace,
                    code,
                    data,
                    raw_log,
                    logs,
                    info,
                    gas_wanted,
                    gas_used,
                    tx,
                    timestamp,
                    events,
                } = cosmos.get_transaction_body(txhash).await?.1;
                println!("Height: {height}");
                println!("Code: {code}");
                println!("Codespace: {codespace}");
                if pretty {
                    match serde_json::from_str::<serde_json::Value>(&raw_log) {
                        Err(_) => println!("Raw log is not JSON: {raw_log}"),
                        Ok(raw_log) => serde_json::to_writer_pretty(std::io::stdout(), &raw_log)?,
                    }
                } else {
                    println!("Raw log: {raw_log}");
                }
                println!("Info: {info}");
                println!("Gas wanted: {gas_wanted}");
                println!("Gas used: {gas_used}");
                println!("Timestamp: {timestamp}");
                if complete {
                    println!("Data: {data}");
                    for (idx, log) in logs.into_iter().enumerate() {
                        println!("Log #{idx}: {log:?}");
                    }
                    for (idx, event) in events.into_iter().enumerate() {
                        println!("Event #{idx}: {event:?}");
                    }
                }
                if let Some(tx) = tx {
                    println!("Encoded length: {}", tx.encoded_len());
                }
            }
            Subcommand::ListTxsFor {
                address,
                limit,
                offset,
            } => {
                let cosmos = opt.network_opt.build().await?;
                for txhash in cosmos.list_transactions_for(address, limit, offset).await? {
                    println!("{txhash}");
                }
            }
            Subcommand::GenerateShellCompletions { shell } => {
                clap_complete::generate(
                    shell,
                    &mut Subcommand::command(),
                    "cosmos",
                    &mut std::io::stdout(),
                );
            }
            Subcommand::ShowBlock { height } => {
                let cosmos = opt.network_opt.build().await?;
                let BlockInfo {
                    height,
                    timestamp,
                    txhashes,
                    block_hash,
                    chain_id,
                } = cosmos.get_block_info(height).await?;
                println!("Chain ID: {chain_id}");
                println!("Height: {height}");
                println!("Timestamp: {timestamp}");
                println!("Block hash: {block_hash}");
                for (idx, txhash) in txhashes.into_iter().enumerate() {
                    println!("Transaction #{}: {txhash}", idx + 1);
                }
            }
            Subcommand::ChangeAddressType {
                orig,
                hrp: address_type,
            } => {
                println!("{}", orig.with_hrp(address_type));
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
                let cosmos = opt.network_opt.build().await?;
                chain::go(inner, cosmos).await?;
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
        }

        Ok(())
    }
}

fn gen_wallet(hrp: AddressHrp) -> Result<()> {
    let phrase = cosmos::SeedPhrase::random();
    let wallet = phrase.with_hrp(hrp)?;
    println!("Mnemonic: {}", phrase.phrase());
    println!("Address: {wallet}");
    Ok(())
}
