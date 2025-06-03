use anyhow::Result;
use cosmos::{HasAddress, HasAddressHrp, TxBuilder};

use crate::cli::TxOpt;

#[derive(clap::Parser)]
pub(crate) enum Subcommand {
    /// Print information about a pool
    Pool {
        /// Asset to check
        asset: String,
    },
    /// Print information about all pools
    Pools {},
    /// Withdraw secured assets
    Withdraw {
        chain: String,
        symbol: String,
        amount: u128,
        destination: String,
        #[clap(flatten)]
        tx_opt: TxOpt,
    },
}

pub(crate) async fn go(opt: crate::cli::Opt, inner: Subcommand) -> Result<()> {
    let cosmos = opt.network_opt.build().await?;
    match inner {
        Subcommand::Pool { asset } => {
            let x = cosmos.rujira_pool(asset).await?;
            println!("{x:#?}");
        }
        Subcommand::Pools {} => {
            let x = cosmos.rujira_pools().await?;
            println!("{x:#?}");
        }
        Subcommand::Withdraw {
            chain,
            symbol,
            amount,
            destination,
            tx_opt,
        } => {
            let wallet = tx_opt.get_wallet(cosmos.get_address_hrp())?;
            let mut builder = TxBuilder::default();
            builder.add_message(cosmos::rujira::MsgDeposit {
                chain,
                symbol,
                amount,
                destination,
                signer: wallet.get_address(),
            });
            let res = builder.sign_and_broadcast(&cosmos, &wallet).await?;
            println!("txhash: {}", res.txhash);
        }
    }

    Ok(())
}
