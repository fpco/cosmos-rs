use std::path::PathBuf;

use anyhow::Result;
use cosmos::{
    clap::CosmosOpt, error::WalletError, Address, AddressHrp, ContractAdmin, RawAddress,
    SeedPhrase, Wallet,
};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

use crate::{authz, chain, contract, nft, parsed_coin::ParsedCoin, tokenfactory};

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
    /// Show config
    ShowConfig {},
    /// Upload contract
    StoreCode {
        #[clap(flatten)]
        tx_opt: TxOpt,
        file: PathBuf,
    },
    /// Instantiate contract
    InstantiateContract {
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
    /// Print balances
    PrintBalances {
        /// Address on COSMOS blockchain
        address: Address,
        /// Optional height to do the query at
        #[clap(long)]
        height: Option<u64>,
    },
    /// Query contract
    QueryContract {
        /// Contract address
        address: Address,
        /// Query (in JSON)
        query: String,
        /// Optional Height. Use latest if not passed.
        height: Option<u64>,
    },
    /// Look up a raw value in the contract's storage
    RawQueryContract {
        /// Contract address
        address: Address,
        /// Key
        key: String,
        /// Optional Height. Use latest if not passed.
        height: Option<u64>,
    },
    /// Migrate contract
    MigrateContract {
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
    ExecuteContract {
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
    SimulateContract {
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
    /// Generate wallet
    GenWallet {
        /// Address type, supports any valid Human Readable Part like cosmos, osmo, or juno
        address_type: AddressHrp,
    },
    /// Print the address for the given phrase
    PrintAddress {
        /// HRP (human readable part) of the address, e.g. osmo, inj
        hrp: AddressHrp,
        /// Phrase
        phrase: SeedPhrase,
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
    /// Get contract metadata
    ContractInfo { contract: Address },
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
    /// Get the contract history
    ContractHistory { contract: Address },
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
    /// Print the address for a different chain
    ChangeAddressType {
        /// Original address
        orig: RawAddress,
        /// Destination address HRP (human-readable part)
        hrp: AddressHrp,
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
    /// Code ID operations
    Code {
        #[clap(flatten)]
        opt: crate::code::Opt,
    },
}
