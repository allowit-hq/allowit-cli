//! `allowit wallet show`.
use crate::{ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit wallet show
  allowit wallet show --wallet WALLET --json";

/// Show the wallet: its roles, asset and state
#[derive(clap::Args, Debug)]
pub(crate) struct Args {}

pub(crate) fn run(_: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let wallet = cx.wallet(rail)?;
    let vendor = cx.vendor()?;
    vendor.wallet_show(cx, wallet.as_deref())
}
