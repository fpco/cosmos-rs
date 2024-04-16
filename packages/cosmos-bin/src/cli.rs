use anyhow::Result;
use cosmos::{clap::CosmosOpt, error::WalletError, Address, AddressHrp, SeedPhrase, Wallet};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

use crate::{authz, chain, contract, nft, tokenfactory};

/// Command line tool for interacting with Cosmos chains
#[derive(clap::Parser)]
pub(crate) struct Cmd {
    #[clap(flatten)]
    pub(crate) opt: Opt,
    #[clap(subcommand)]
    pub(crate) subcommand: Subcommand,
}

#[derive(clap::Parser)]
pub(crate) struct Opt {
    #[clap(flatten)]
    pub(crate) network_opt: CosmosOpt,
    /// Turn on verbose output
    #[clap(long, short, global = true)]
    verbose: bool,
}

impl Opt {
    pub(crate) fn init_logger(&self) -> Result<()> {
        let mut filter = EnvFilter::from_default_env().add_directive(Level::INFO.into());

        if self.verbose {
            filter = filter.add_directive("cosmos=debug".parse()?);
            filter = filter.add_directive(format!("{}=debug", env!("CARGO_CRATE_NAME")).parse()?);
        };

        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::Layer::default()
                .with_writer(std::io::stderr)
                .and_then(filter),
        );

        subscriber.init();
        Ok(())
    }
}

#[derive(clap::Parser)]
pub(crate) struct TxOpt {
    /// Mnemonic phrase
    #[clap(long, env = "COSMOS_WALLET")]
    pub(crate) wallet: SeedPhrase,
    /// Memo to put on transaction
    #[clap(long)]
    pub(crate) memo: Option<String>,
}

impl TxOpt {
    pub(crate) fn get_wallet(&self, hrp: AddressHrp) -> Result<Wallet, WalletError> {
        self.wallet.with_hrp(hrp)
    }
}

#[derive(clap::Parser)]
pub(crate) enum Subcommand {
    /// Bank module operations
    Bank {
        #[clap(flatten)]
        opt: crate::bank::Opt,
    },
    /// Management of wallets and seed phrases
    Wallet {
        #[clap(flatten)]
        opt: crate::wallet::Opt,
    },
    /// Show config
    ShowConfig {},
    /// Show transaction details
    ShowTx {
        txhash: String,
        /// Show all the data in the transaction?
        #[clap(long)]
        complete: bool,
        /// Pretty-print JSON output?
        #[clap(long)]
        pretty: bool,
    },
    /// List transactions for a given wallet
    ListTxsFor {
        address: Address,
        /// Maximum number of transactions to return
        #[clap(long)]
        limit: Option<u64>,
        /// Offset
        #[clap(long)]
        offset: Option<u64>,
    },
    /// Generate bash shell completion script
    GenerateShellCompletions {
        /// Which shell to generate for
        #[clap(default_value_t = clap_complete::Shell::Bash)]
        shell: clap_complete::Shell,
    },
    /// Show block metadata and transaction hashes within the block
    ShowBlock {
        /// Height of the block to show
        height: i64,
    },
    /// NFT focused subcommands
    Nft {
        #[clap(flatten)]
        opt: nft::Opt,
        #[clap(subcommand)]
        subcommand: nft::Subcommand,
    },
    /// Smart contract subcommands
    Contract {
        #[clap(flatten)]
        opt: contract::Opt,
    },
    /// Information about the chain
    Chain {
        #[clap(flatten)]
        opt: chain::Opt,
    },

    /// Tokenfactory operations
    TokenFactory {
        /// Mnemonic phrase
        #[clap(long, env = "COSMOS_WALLET")]
        wallet: SeedPhrase,

        #[clap(subcommand)]
        cmd: tokenfactory::Command,
    },
    /// Authz operations
    Authz {
        #[clap(flatten)]
        opt: authz::Opt,
    },
    /// CW3 multisig operations
    Cw3 {
        #[clap(flatten)]
        opt: crate::cw3::Opt,
    },
}
