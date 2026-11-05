//! `allowit keys show NAME`.
use crate::{ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit keys show agent
  allowit keys show agent --json";

/// Show a key's public metadata
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Key name
    pub name: String,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let key = cx.key(&args.name)?;
    cx.out.value(&super::view(&key));
    Ok(0)
}
