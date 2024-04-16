use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use cosmos::{Address, BlockInfo, Cosmos, TxResponseExt};

#[derive(clap::Parser)]
pub(crate) struct Opt {
    #[clap(subcommand)]
    sub: Subcommand,
}

#[derive(clap::Parser)]
pub(crate) enum Subcommand {
    /// Find the first block after the given timestamp
    FirstBlockAfter {
        #[clap(long)]
        timestamp: DateTime<Utc>,
        #[clap(long)]
        earliest: Option<i64>,
    },
    /// Get account number and sequence number for the given address
    AccountInfo { address: Address },
    /// Get the code ID from the given transaction
    CodeIdFromTx { txhash: String },
    /// Get the contract address instantiated in a given transaction
    ContractAddressFromTx { txhash: String },
    /// Check that all transaction data is available on an archive node
    ArchiveCheck {
        #[clap(long)]
        start_block: i64,
        #[clap(long)]
        end_block: Option<i64>,
    },
    /// Print a CSV file with gas usage in a range of blocks
    BlockGasReport {
        #[clap(long)]
        start_block: i64,
        #[clap(long)]
        end_block: i64,
        #[clap(long)]
        dest: PathBuf,
    },
    /// Print the latest block info
    Latest {},
    /// Print Osmosis-specific epoch information.
    Epoch {},
    /// Print Osmosis-specific txfees information.
    TxFees {},
}

pub(crate) async fn go(Opt { sub }: Opt, cosmos: Cosmos) -> Result<()> {
    match sub {
        Subcommand::FirstBlockAfter {
            timestamp,
            earliest,
        } => first_block_after(cosmos, timestamp, earliest).await,
        Subcommand::AccountInfo { address } => account_info(cosmos, address).await,
        Subcommand::CodeIdFromTx { txhash } => code_id_from_tx(cosmos, txhash).await,
        Subcommand::ContractAddressFromTx { txhash } => {
            contract_address_from_tx(cosmos, txhash).await
        }
        Subcommand::ArchiveCheck {
            start_block,
            end_block,
        } => archive_check(cosmos, start_block, end_block).await,
        Subcommand::BlockGasReport {
            start_block,
            end_block,
            dest,
        } => block_gas_report(cosmos, start_block, end_block, &dest).await,
        Subcommand::Latest {} => latest(cosmos).await,
        Subcommand::Epoch {} => epoch(cosmos).await,
        Subcommand::TxFees {} => txfees(cosmos).await,
    }
}

async fn first_block_after(
    cosmos: Cosmos,
    timestamp: DateTime<Utc>,
    earliest: Option<i64>,
) -> Result<()> {
    let block = cosmos.first_block_after(timestamp, earliest).await?;
    println!("{block}");
    Ok(())
}

async fn account_info(cosmos: Cosmos, address: Address) -> Result<()> {
    let base_account = cosmos.get_base_account(address).await?;
    tracing::info!("Account number: {}", base_account.account_number);
    tracing::info!("Sequence number: {}", base_account.sequence);
    Ok(())
}

async fn code_id_from_tx(cosmos: Cosmos, txhash: String) -> Result<()> {
    let (_, txres) = cosmos.get_transaction_body(txhash).await?;
    let code_id = txres.parse_first_stored_code_id()?;
    tracing::info!("Code ID: {code_id}");
    Ok(())
}

async fn contract_address_from_tx(cosmos: Cosmos, txhash: String) -> Result<()> {
    let (_, tx) = cosmos.wait_for_transaction(&txhash).await?;
    let addrs = tx.parse_instantiated_contracts()?;

    anyhow::ensure!(
        !addrs.is_empty(),
        "No contract addresses found in transaction {txhash}"
    );
    addrs
        .into_iter()
        .for_each(|contract| tracing::info!("Contract address: {contract}"));
    Ok(())
}

async fn archive_check(cosmos: Cosmos, start_block: i64, end_block: Option<i64>) -> Result<()> {
    let end_block = match end_block {
        Some(end_block) => end_block,
        None => {
            let end_block = cosmos.get_latest_block_info().await?.height;
            tracing::info!("Checking until block height {end_block}");
            end_block
        }
    };
    anyhow::ensure!(end_block >= start_block);
    for block_height in start_block..=end_block {
        tracing::info!("Checking block {block_height}");
        match cosmos.get_block_info(block_height).await {
            Ok(block) => {
                for txhash in block.txhashes {
                    if let Err(e) = cosmos.get_transaction_body(&txhash).await {
                        tracing::error!("Error while getting transaction {txhash}: {e:?}");
                        println!("Missing transaction: {txhash} in block: {block_height}");
                    }
                }
            }
            Err(e) => {
                tracing::error!("Error while processing block {block_height}: {e:?}");
                println!("Missing block: {block_height}");
            }
        };
    }
    Ok(())
}

async fn block_gas_report(
    cosmos: Cosmos,
    start_block: i64,
    end_block: i64,
    dest: &PathBuf,
) -> Result<()> {
    let mut csv = csv::Writer::from_path(dest)?;
    #[derive(serde::Serialize)]
    struct Record {
        block: i64,
        timestamp: DateTime<Utc>,
        gas_used: i64,
        gas_wanted: i64,
        txcount: usize,
    }
    for height in start_block..=end_block {
        let block = cosmos.get_block_info(height).await?;
        let mut gas_used = 0;
        let mut gas_wanted = 0;
        let txcount = block.txhashes.len();
        for txhash in block.txhashes {
            let (_, tx) = cosmos.get_transaction_body(txhash).await?;
            gas_used += tx.gas_used;
            gas_wanted += tx.gas_wanted;
        }
        csv.serialize(Record {
            block: block.height,
            timestamp: block.timestamp,
            gas_used,
            gas_wanted,
            txcount,
        })?;
        csv.flush()?;
    }
    csv.flush()?;
    Ok(())
}

async fn latest(cosmos: Cosmos) -> std::result::Result<(), anyhow::Error> {
    let BlockInfo {
        height,
        timestamp,
        txhashes,
        block_hash,
        chain_id,
    } = cosmos.get_latest_block_info().await?;
    println!("Chain ID: {chain_id}");
    println!("Height: {height}");
    println!("Timestamp: {timestamp}");
    println!("Block hash: {block_hash}");
    for (idx, txhash) in txhashes.into_iter().enumerate() {
        println!("Transaction #{}: {txhash}", idx + 1);
    }
    Ok(())
}

async fn epoch(cosmos: Cosmos) -> std::result::Result<(), anyhow::Error> {
    let epoch = cosmos.get_osmosis_epoch_info().await?;
    println!("{epoch:?}");
    let epoch = epoch.summarize();
    println!("{epoch:?}");
    Ok(())
}

async fn txfees(cosmos: Cosmos) -> std::result::Result<(), anyhow::Error> {
    let txfees = cosmos.get_osmosis_txfees_info().await?;
    println!("eip base fee: {}", txfees.eip_base_fee);
    Ok(())
}
