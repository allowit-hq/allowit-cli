//! `allowit wallet use ADDR`.
use crate::{ctx::Ctx, error::Result, vendors::address};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit wallet use WALLET";

/// Make an address the current rail's default wallet
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// The wallet address
    pub address: String,
    /// Exact cached public profile version
    #[arg(long)]
    pub profile: Option<String>,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    address::check_wallet(rail.family, &args.address)?;
    if let Some(version) = &args.profile {
        let agent = cx.agent(rail)?;
        allowit_sdk_engine::profiles::Cache::open(&cx.dir()?)?.select(
            &allowit_sdk_engine::profiles::Selector {
                network: rail.id.into(),
                wallet: Some(args.address.clone()),
                agent: Some(agent.address),
                version: Some(version.clone()),
                ..Default::default()
            },
        )?;
    }
    cx.settings()?.update(|config| {
        let r = config.rail_mut(rail.id);
        r.wallet = Some(args.address.clone());
        if !r.wallets.contains(&args.address) {
            r.wallets.push(args.address.clone());
        }
        Ok(())
    })?;
    cx.out
        .value(&serde_json::json!({"rail": rail.id, "wallet": args.address}));
    Ok(0)
}
