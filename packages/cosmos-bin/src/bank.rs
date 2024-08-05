use anyhow::Result;
use cosmos::{
    error::{QueryError, QueryErrorDetails},
    proto::cosmos::bank::v1beta1::MsgSend,
    Address, Coin, Cosmos, HasAddress, HasAddressHrp, ParsedCoin, TxBuilder,
};

use crate::cli::TxOpt;

#[derive(clap::Parser)]
pub(crate) struct Opt {
    #[clap(subcommand)]
    sub: Subcommand,
}

#[derive(clap::Parser)]
enum Subcommand {
    /// Print balances
    PrintBalances {
        /// Address on COSMOS blockchain
        address: Address,
        /// Optional height to do the query at
        #[clap(long)]
        height: Option<u64>,
    },
    /// Send coins to the given address
    Send {
        #[clap(flatten)]
        tx_opt: TxOpt,
        /// Destination address
        dest: Address,
        /// Coins to send
        coins: Vec<ParsedCoin>,
    },
    /// Collect all funds from numbered, indexed wallets
    CollectIndexed {
        #[clap(flatten)]
        tx_opt: TxOpt,
        /// Destination address
        dest: Address,
        /// How many non-existent wallets to tolerate
        #[clap(long, default_value_t = 4)]
        missing_tolerance: u32,
    },
}

pub(crate) async fn go(cosmos: Cosmos, Opt { sub }: Opt) -> Result<()> {
    match sub {
        Subcommand::PrintBalances { address, height } => {
            let balances = cosmos.at_height(height).all_balances(address).await?;
            for Coin { denom, amount } in &balances {
                println!("{amount}{denom}");
            }
            if balances.is_empty() {
                println!("0");
            }
        }
        Subcommand::Send {
            tx_opt,
            dest,
            coins,
        } => {
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
        Subcommand::CollectIndexed {
            tx_opt,
            dest,
            missing_tolerance,
        } => {
            let mut missed = 0;
            for index in 5.. {
                let wallet = tx_opt
                    .wallet
                    .clone()
                    .with_numbered(index, cosmos.get_address_hrp())?;
                tracing::info!("Processing index #{index}, wallet address: {wallet}");
                if wallet.get_address() == dest {
                    tracing::info!("Skipping same destination address");
                }
                match cosmos.get_base_account(wallet.get_address()).await {
                    Ok(_) => (),
                    Err(cosmos::Error::Query(QueryError {
                        query: QueryErrorDetails::NotFound(s),
                        ..
                    })) => {
                        tracing::info!("Wallet not found on chain: {s}. Missed: {missed}. Missing tolerance: {missing_tolerance}.");
                        missed += 1;
                        if missed >= missing_tolerance {
                            return Ok(());
                        }
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                };
                missed = 0;
                let balances = cosmos.all_balances(wallet.get_address()).await?;
                tracing::info!("Balances: {balances:?}");
                let gas_denom = cosmos.get_gas_coin();
                let from_address = wallet.get_address();
                let make_tx_builder = move |gas_to_hold: Option<u64>| {
                    let mut to_send = vec![];
                    for Coin { denom, amount } in balances.iter().take(3) {
                        let mut amount = amount.parse::<u64>()?;
                        if denom == gas_denom {
                            if let Some(gas_to_hold) = gas_to_hold {
                                if amount <= gas_to_hold {
                                    continue;
                                }
                                amount -= gas_to_hold;
                            }
                        }
                        to_send.push(Coin {
                            denom: denom.clone(),
                            amount: amount.to_string(),
                        })
                    }
                    let mut tx_builder = TxBuilder::default();
                    tx_builder.add_message(MsgSend {
                        from_address: from_address.get_address_string(),
                        to_address: dest.get_address_string(),
                        amount: to_send,
                    });
                    anyhow::Ok(tx_builder)
                };
                let txbuilder = make_tx_builder(None)?;
                let res = txbuilder.simulate(&cosmos, &[wallet.get_address()]).await?;
                let gas_to_use = res.gas_used as f64 * cosmos.get_current_gas_multiplier();
                // add 2 for a bit of a buffer
                let gas_fee = (gas_to_use * cosmos.get_base_gas_price().await) as u64 + 100;
                let txbuilder = make_tx_builder(Some(gas_fee))?;
                let balances = cosmos.all_balances(wallet.get_address()).await?;
                println!(
                    "gas_used {}. gas_to_use: {}. gas_fee: {}. balances: {balances:?}",
                    res.gas_used, gas_to_use, gas_fee
                );
                let res = txbuilder
                    .sign_and_broadcast_with_gas(&cosmos, &wallet, gas_to_use as u64)
                    .await?;
                tracing::info!("Funds sent in {}", res.txhash);
            }
        }
    }
    Ok(())
}
