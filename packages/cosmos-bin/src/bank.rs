use anyhow::Result;
use cosmos::{
    proto::cosmos::bank::v1beta1::MsgSend, Address, Coin, Cosmos, HasAddress, HasAddressHrp,
    TxBuilder,
};

use crate::{cli::TxOpt, parsed_coin::ParsedCoin};

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
    SendCoins {
        #[clap(flatten)]
        tx_opt: TxOpt,
        /// Destination address
        dest: Address,
        /// Coins to send
        coins: Vec<ParsedCoin>,
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
        Subcommand::SendCoins {
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
    }
    Ok(())
}
