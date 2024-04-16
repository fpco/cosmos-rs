mod authz;
mod chain;
mod cli;
mod code;
mod contract;
mod cw3;
mod my_duration;
mod nft;
mod parsed_coin;
mod tokenfactory;

use std::{io::Write, str::FromStr};

use anyhow::Result;
use clap::{CommandFactory, Parser};
use cli::Subcommand;
use cosmos::{
    proto::{
        cosmos::{bank::v1beta1::MsgSend, base::abci::v1beta1::TxResponse},
        cosmwasm::wasm::v1::{
            ContractCodeHistoryEntry, ContractInfo, MsgExecuteContract,
            QueryContractHistoryResponse,
        },
        traits::Message,
    },
    AddressHrp, BlockInfo, Coin, HasAddress, HasAddressHrp, TxBuilder,
};
use parsed_coin::ParsedCoin;

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
            Subcommand::StoreCode { tx_opt, file } => {
                let cosmos = opt.network_opt.build().await?;
                let address_type = cosmos.get_address_hrp();
                let wallet = tx_opt.get_wallet(address_type)?;
                let codeid = cosmos.store_code_path(&wallet, &file).await?;
                println!("Code ID: {codeid}");
            }
            Subcommand::InstantiateContract {
                tx_opt,
                code_id,
                label,
                msg,
                admin,
            } => {
                let cosmos = opt.network_opt.build().await?;
                let address_type = cosmos.get_address_hrp();
                let contract = cosmos
                    .make_code_id(code_id)
                    .instantiate_rendered(
                        &tx_opt.get_wallet(address_type)?,
                        label,
                        vec![],
                        msg,
                        admin,
                    )
                    .await?;
                println!("Contract: {contract}");
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
            Subcommand::QueryContract {
                address,
                query,
                height,
            } => {
                let cosmos = opt.network_opt.build().await?.at_height(height);
                let x = cosmos
                    .make_contract(address)
                    .query_rendered_bytes(query)
                    .await?;
                let stdout = std::io::stdout();
                let mut stdout = stdout.lock();
                stdout.write_all(&x)?;
                stdout.write_all(b"\n")?;
            }
            Subcommand::RawQueryContract {
                address,
                key,
                height,
            } => {
                let cosmos = opt.network_opt.build().await?.at_height(height);
                let x = cosmos.make_contract(address).query_raw(key).await?;
                let stdout = std::io::stdout();
                let mut stdout = stdout.lock();
                stdout.write_all(&x)?;
                stdout.write_all(b"\n")?;
            }
            Subcommand::MigrateContract {
                tx_opt,
                address,
                code_id,
                msg,
            } => {
                let cosmos = opt.network_opt.build().await?;
                let address_type = cosmos.get_address_hrp();
                let contract = cosmos.make_contract(address);
                contract
                    .migrate_binary(&tx_opt.get_wallet(address_type)?, code_id, msg)
                    .await?;
            }
            Subcommand::ExecuteContract {
                tx_opt,
                address,
                msg,
                funds: amount,
                skip_simulate,
            } => {
                let cosmos = opt.network_opt.build().await?;
                let address_type = cosmos.get_address_hrp();
                let contract = cosmos.make_contract(address);
                let amount = match amount {
                    Some(funds) => {
                        let coin = ParsedCoin::from_str(&funds)?.into();
                        vec![coin]
                    }
                    None => vec![],
                };
                let wallet = tx_opt.get_wallet(address_type)?;

                let mut tx_builder = TxBuilder::default();
                tx_builder.add_message(MsgExecuteContract {
                    sender: wallet.get_address_string(),
                    contract: contract.get_address_string(),
                    msg: msg.into_bytes(),
                    funds: amount,
                });

                let tx = match skip_simulate {
                    Some(gas_to_request) => {
                        tx_builder
                            .sign_and_broadcast_with_gas(&cosmos, &wallet, gas_to_request)
                            .await?
                    }
                    None => tx_builder.sign_and_broadcast(&cosmos, &wallet).await?,
                };

                println!("Transaction hash: {}", tx.txhash);
                println!("Raw log: {}", tx.raw_log);
                tracing::debug!("{tx:?}");
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
            Subcommand::ContractInfo { contract } => {
                let cosmos = opt.network_opt.build().await?;
                let ContractInfo {
                    code_id,
                    creator,
                    admin,
                    label,
                    created: _,
                    ibc_port_id: _,
                    extension: _,
                } = cosmos.make_contract(contract).info().await?;
                println!("code_id: {code_id}");
                println!("creator: {creator}");
                println!("admin: {admin}");
                println!("label: {label}");
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
            Subcommand::ContractHistory { contract } => {
                let cosmos = opt.network_opt.build().await?;
                let QueryContractHistoryResponse {
                    entries,
                    pagination: _,
                } = cosmos.make_contract(contract).history().await?;
                for ContractCodeHistoryEntry {
                    operation,
                    code_id,
                    updated,
                    msg,
                } in entries
                {
                    println!("Operation: {operation}. Code ID: {code_id}. Updated: {updated:?}. Message: {:?}", String::from_utf8(msg))
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
            Subcommand::SimulateContract {
                sender,
                memo,
                address,
                msg,
                funds,
            } => {
                let cosmos = opt.network_opt.build().await?;
                let address_type = cosmos.get_address_hrp();
                let contract = cosmos.make_contract(address);
                let amount = match funds {
                    Some(funds) => {
                        let coin = ParsedCoin::from_str(&funds)?.into();
                        vec![coin]
                    }
                    None => vec![],
                };
                let simres = contract
                    .simulate_binary(sender.with_hrp(address_type), amount, msg, memo)
                    .await?;
                println!("{simres:?}");
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
            Subcommand::Code { opt: inner } => {
                let cosmos = opt.network_opt.build().await?;
                code::go(cosmos, inner).await?;
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
