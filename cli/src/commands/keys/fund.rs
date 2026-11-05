//! `allowit keys fund NAME`.
use crate::{ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit --rail __ALLOWIT_EXAMPLE_RAIL__ keys fund agent";

/// Fund a key's account through the selected rail's test faucet
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Key name
    pub name: String,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let key = cx.key(&args.name)?;
    let vendor = cx.vendor()?;
    vendor.keys_fund(cx, &key)
}
