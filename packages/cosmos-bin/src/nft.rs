use std::{collections::BTreeMap, fs::File, path::PathBuf, sync::Arc};

use anyhow::Result;
use async_channel::RecvError;
use cosmos::{Address, Contract, Cosmos, HasAddress, HasAddressHrp, TxBuilder};
use cosmwasm_std::Uint64;
use parking_lot::Mutex;
use tokio::task::JoinSet;

use crate::cli::TxOpt;

#[derive(clap::Parser)]
pub(crate) enum Subcommand {
    /// Transfer all NFTs to the given destination wallet
    TransferAll {
        /// NFT contract address
        #[clap(long, env = "NFT_CONTRACT")]
        nft_contract: Address,
        #[clap(long)]
        dest: Address,
        #[clap(flatten)]
        tx_opt: TxOpt,
    },
    /// Produce a CSV file with owner information for all NFTs in a contract
    OwnersCsv {
        /// NFT contracts to query
        #[clap(long, required = true)]
        nft_contract: Vec<Address>,
        /// Output file
        #[clap(long)]
        output: PathBuf,
        /// Worker tasks
        #[clap(long, default_value_t = 8)]
        workers: usize,
    },
}

pub(super) async fn go(sub: Subcommand, cosmos: Cosmos) -> Result<()> {
    match sub {
        Subcommand::TransferAll {
            nft_contract,
            dest,
            tx_opt,
        } => {
            let contract = cosmos.make_contract(nft_contract);
            let wallet = tx_opt.get_wallet(cosmos.get_address_hrp())?;
            loop {
                let Tokens { tokens } = contract
                    .query(&NftQuery::Tokens {
                        owner: wallet.get_address(),
                        limit: 30,
                    })
                    .await?;
                if tokens.is_empty() {
                    tracing::info!("No more tokens remaining");
                    break;
                }
                let count = tokens.len();

                let mut builder = TxBuilder::default();
                for token_id in tokens {
                    builder.add_execute_message(
                        &contract,
                        &wallet,
                        vec![],
                        NftExec::TransferNft {
                            token_id,
                            recipient: dest,
                        },
                    )?;
                }
                let res = builder.sign_and_broadcast(&cosmos, &wallet).await?;
                tracing::info!(
                    "Transferred {count} {} in {}",
                    if count == 1 { "NFT" } else { "NFTs" },
                    res.txhash
                );
            }
        }
        Subcommand::OwnersCsv {
            nft_contract,
            output,
            workers,
        } => {
            owners_csv(cosmos, nft_contract, workers, output).await?;
        }
    }
    Ok(())
}

enum WorkItem {
    GetTokens {
        nft_contract: Contract,
        start_after: Option<u64>,
        tx: async_channel::Sender<WorkItem>,
    },
    GetOwner {
        nft_contract: Contract,
        token_id: u64,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
struct OwnerRecord {
    contract: Address,
    owner: Address,
    token_id: u64,
}

async fn run_worker(
    rx: async_channel::Receiver<WorkItem>,
    csv: Arc<Mutex<csv::Writer<File>>>,
) -> Result<()> {
    loop {
        match rx.recv().await {
            Ok(WorkItem::GetTokens {
                nft_contract,
                start_after,
                tx,
            }) => {
                match start_after {
                    None => {
                        println!("Getting first batch of tokens for contract {nft_contract}")
                    }
                    Some(token_id) => {
                        println!("Getting tokens after ID {token_id} for contract {nft_contract}")
                    }
                }
                let AllTokensResp { tokens } = nft_contract
                    .query(NftQuery::AllTokens {
                        start_after: start_after.map(Uint64::new),
                    })
                    .await?;
                let Some(last) = tokens.last().copied() else {
                    continue;
                };
                for token in tokens {
                    tx.send(WorkItem::GetOwner {
                        nft_contract: nft_contract.clone(),
                        token_id: token.u64(),
                    })
                    .await?;
                }
                tx.clone()
                    .send(WorkItem::GetTokens {
                        nft_contract,
                        start_after: Some(last.u64()),
                        tx,
                    })
                    .await?;
            }
            Ok(WorkItem::GetOwner {
                nft_contract,
                token_id,
            }) => {
                let OwnerOfResp { owner } = nft_contract
                    .query(NftQuery::OwnerOf {
                        token_id: Uint64::new(token_id),
                    })
                    .await?;
                let mut csv = csv.lock();
                csv.serialize(&OwnerRecord {
                    contract: nft_contract.get_address(),
                    owner,
                    token_id,
                })?;
                csv.flush()?;
            }
            Err(RecvError) => break Ok(()),
        }
    }
}

async fn owners_csv(
    cosmos: Cosmos,
    nft_contract: Vec<Address>,
    workers: usize,
    output: PathBuf,
) -> Result<()> {
    type Results = BTreeMap<Address, BTreeMap<u64, Address>>;
    let mut results = Results::new();

    if output.exists() {
        for record in csv::Reader::from_path(&output)?.into_deserialize() {
            let OwnerRecord {
                contract,
                owner,
                token_id,
            } = record?;
            results.entry(contract).or_default().insert(token_id, owner);
        }
    }

    let mut csv = csv::Writer::from_path(&output)?;
    for (contract, results) in &results {
        for (token_id, owner) in results {
            csv.serialize(&OwnerRecord {
                contract: *contract,
                owner: *owner,
                token_id: *token_id,
            })?;
        }
    }
    csv.flush()?;
    let csv = Arc::new(Mutex::new(csv));

    let mut set = JoinSet::new();
    let (tx, rx) = async_channel::bounded(workers * 4);
    for _ in 0..workers {
        set.spawn(run_worker(rx.clone(), csv.clone()));
    }

    for nft_contract in nft_contract {
        let mut start_after = None;

        if let Some(existing) = results.get(&nft_contract) {
            for token_id in 1.. {
                if existing.contains_key(&token_id) {
                    start_after = Some(token_id);
                } else {
                    break;
                }
            }
        }

        tx.send(WorkItem::GetTokens {
            nft_contract: cosmos.make_contract(nft_contract),
            start_after,
            tx: tx.clone(),
        })
        .await?;
    }
    std::mem::drop(results);
    std::mem::drop(tx);

    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(())) => (),
            Ok(Err(err)) => {
                set.abort_all();
                return Err(err);
            }
            Err(err) => {
                set.abort_all();
                return Err(err.into());
            }
        }
    }

    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum NftQuery {
    Tokens { owner: Address, limit: u32 },
    AllTokens { start_after: Option<Uint64> },
    OwnerOf { token_id: Uint64 },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum NftExec {
    TransferNft {
        token_id: String,
        recipient: Address,
    },
}

#[derive(serde::Deserialize)]
struct Tokens {
    tokens: Vec<String>,
}

#[derive(serde::Deserialize)]
struct AllTokensResp {
    tokens: Vec<Uint64>,
}

#[derive(serde::Deserialize)]
struct OwnerOfResp {
    owner: Address,
}
