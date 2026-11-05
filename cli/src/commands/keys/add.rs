//! `allowit keys add NAME --public-key ADDR`.
use crate::{ctx::Ctx, error::Result, settings::keys};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit keys add owner --public-key PUBLIC_KEY";

/// Record a public key that this device cannot sign for, such as the owner's
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Key name
    pub name: String,
    /// Public key or account address accepted by the selected rail
    #[arg(long, value_name = "ADDR")]
    pub public_key: String,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let dir = cx.dir()?;
    let key = keys::add(&dir, &args.name, rail.id, &args.public_key)?;
    cx.out.value(&super::view(&key));
    Ok(0)
}
