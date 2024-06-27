use std::{io::Write, path::PathBuf, str::FromStr};

use anyhow::Result;
use cosmos::{
    proto::cosmwasm::wasm::v1::{
        ContractCodeHistoryEntry, ContractInfo, MsgExecuteContract, QueryContractHistoryResponse,
    },
    Address, ContractAdmin, Cosmos, HasAddress, HasAddressHrp, ParsedCoin, RawAddress, TxBuilder,
};

use crate::cli::TxOpt;

#[derive(clap::Parser)]
pub(crate) struct Opt {
    #[clap(subcommand)]
    subcommand: Subcommand,
}

#[derive(clap::Parser)]
enum Subcommand {
    /// Update the administrator on a contract
    UpdateAdmin {
        /// Smart contract address
        #[clap(long, env = "CONTRACT")]
        contract: Address,
        #[clap(long)]
        new_admin: Address,
        #[clap(flatten)]
        tx_opt: TxOpt,
    },
    /// Simulate migrating a contract, but don't actually do it
    SimulateMigrate {
        /// Smart contract address
        #[clap(long, env = "CONTRACT")]
        contract: Address,
        #[clap(long, env = "COSMOS_SENDER")]
        sender: Address,
        /// Memo to put on transaction
        #[clap(long)]
        memo: Option<String>,
        /// Migration message (JSON)
        msg: String,
        /// New code ID
        #[clap(long)]
        code_id: u64,
    },
    /// Upload contract
    StoreCode {
        #[clap(flatten)]
        tx_opt: TxOpt,
        file: PathBuf,
    },
    /// Instantiate contract
    Instantiate {
        #[clap(flatten)]
        tx_opt: TxOpt,
        /// Code to deploy
        code_id: u64,
        /// Label to display
        label: String,
        /// Instantiate message (JSON)
        msg: String,
        /// Administrator set on this contract
        #[clap(long, default_value = "sender")]
        admin: ContractAdmin,
    },
    /// Query contract
    Query {
        /// Contract address
        address: Address,
        /// Query (in JSON)
        query: String,
        /// Optional Height. Use latest if not passed.
        height: Option<u64>,
    },
    /// Look up a raw value in the contract's storage
    RawQuery {
        /// Contract address
        address: Address,
        /// Key
        key: String,
        /// Optional Height. Use latest if not passed.
        height: Option<u64>,
    },
    /// Migrate contract
    Migrate {
        #[clap(flatten)]
        tx_opt: TxOpt,
        /// Contract address
        address: Address,
        /// New code ID
        code_id: u64,
        /// Migrate message (JSON)
        msg: String,
    },
    /// Execute contract
    Execute {
        #[clap(flatten)]
        tx_opt: TxOpt,
        /// Contract address
        address: Address,
        /// Execute message (JSON)
        msg: String,
        /// Funds. Example 100ujunox
        #[clap(long)]
        funds: Option<String>,
        /// Skip the simulate phase and hard-code the given gas request instead
        #[clap(long)]
        skip_simulate: Option<u64>,
    },
    /// Simulate executing a message, but don't actually do it
    Simulate {
        #[clap(long, env = "COSMOS_SENDER")]
        sender: RawAddress,
        /// Memo to put on transaction
        #[clap(long)]
        memo: Option<String>,
        /// Contract address
        address: Address,
        /// Execute message (JSON)
        msg: String,
        /// Funds. Example 100ujunox
        funds: Option<String>,
    },
    /// Get contract metadata
    Info { contract: Address },
    /// Get the contract history
    History { contract: Address },
    /// Download the code for a given code ID
    Download {
        #[clap(long)]
        code_id: u64,
        #[clap(long)]
        dest: PathBuf,
    },
}

pub(crate) async fn go(Opt { subcommand }: Opt, cosmos: Cosmos) -> Result<()> {
    match subcommand {
        Subcommand::UpdateAdmin {
            new_admin,
            tx_opt,
            contract,
        } => {
            let wallet = tx_opt.get_wallet(cosmos.get_address_hrp())?;
            TxBuilder::default()
                .add_update_contract_admin(contract, &wallet, new_admin)
                .sign_and_broadcast(&cosmos, &wallet)
                .await?;
        }
        Subcommand::SimulateMigrate {
            sender,
            memo,
            msg,
            code_id,
            contract,
        } => {
            let mut txbuilder = TxBuilder::default();
            if let Some(memo) = memo {
                txbuilder.set_memo(memo);
            }
            let msg: serde_json::Value = serde_json::from_str(&msg)?;
            txbuilder.add_migrate_message(contract, sender, code_id, msg)?;
            let simres = txbuilder.simulate(&cosmos, &[sender]).await?;
            println!("{simres:?}");
        }
        Subcommand::Download { code_id, dest } => {
            let code = cosmos.make_code_id(code_id);
            let bytes = code.download().await?;
            fs_err::write(&dest, bytes)?;
        }
        Subcommand::StoreCode { tx_opt, file } => {
            let address_type = cosmos.get_address_hrp();
            let wallet = tx_opt.get_wallet(address_type)?;
            let codeid = cosmos.store_code_path(&wallet, &file).await?;
            println!("Code ID: {codeid}");
        }
        Subcommand::Instantiate {
            tx_opt,
            code_id,
            label,
            msg,
            admin,
        } => {
            let address_type = cosmos.get_address_hrp();
            let contract = cosmos
                .make_code_id(code_id)
                .instantiate_rendered(&tx_opt.get_wallet(address_type)?, label, vec![], msg, admin)
                .await?;
            println!("Contract: {contract}");
        }
        Subcommand::Query {
            address,
            query,
            height,
        } => {
            let cosmos = cosmos.at_height(height);
            let x = cosmos
                .make_contract(address)
                .query_rendered_bytes(query)
                .await?;
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            stdout.write_all(&x)?;
            stdout.write_all(b"\n")?;
        }
        Subcommand::RawQuery {
            address,
            key,
            height,
        } => {
            let cosmos = cosmos.at_height(height);
            let x = cosmos.make_contract(address).query_raw(key).await?;
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            stdout.write_all(&x)?;
            stdout.write_all(b"\n")?;
        }
        Subcommand::Migrate {
            tx_opt,
            address,
            code_id,
            msg,
        } => {
            let address_type = cosmos.get_address_hrp();
            let contract = cosmos.make_contract(address);
            contract
                .migrate_binary(&tx_opt.get_wallet(address_type)?, code_id, msg)
                .await?;
        }
        Subcommand::Execute {
            tx_opt,
            address,
            msg,
            funds: amount,
            skip_simulate,
        } => {
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
        Subcommand::Info { contract } => {
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
        Subcommand::History { contract } => {
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
        Subcommand::Simulate {
            sender,
            memo,
            address,
            msg,
            funds,
        } => {
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
    }
    Ok(())
}
