//! `allowit keys generate NAME`.
use crate::{ctx::Ctx, error::Result, settings::keys};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit keys generate agent
  allowit keys generate owner";

/// Generate a private signing key for the selected rail
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Key name
    pub name: String,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let dir = cx.dir()?;
    let key = keys::generate(&dir, &args.name, rail.id)?;
    cx.out.value(&super::view(&key));
    Ok(0)
}
