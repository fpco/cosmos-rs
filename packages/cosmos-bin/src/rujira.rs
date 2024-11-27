use anyhow::Result;

#[derive(clap::Parser)]
pub(crate) enum Subcommand {
    /// Print information about a pool
    Pool {
        /// Asset to check
        asset: String,
    },
    /// Print information about all pools
    Pools {},
}

pub(crate) async fn go(opt: crate::cli::Opt, inner: Subcommand) -> Result<()> {
    match inner {
        Subcommand::Pool { asset } => {
            let cosmos = opt.network_opt.build().await?;
            let x = cosmos.rujira_pool(asset).await?;
            println!("{x:#?}");
        }
        Subcommand::Pools {} => {
            let cosmos = opt.network_opt.build().await?;
            let x = cosmos.rujira_pools().await?;
            println!("{x:#?}");
        }
    }

    Ok(())
}
