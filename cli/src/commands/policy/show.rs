//! `allowit policy show [--source]`.
use crate::{ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit policy show
  allowit policy show --source";

/// Show the wallet's installed policy
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Print only the policy source
    #[arg(long)]
    pub source: bool,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let vendor = cx.vendor()?;
    vendor.policy_show(cx, args.source)
}
