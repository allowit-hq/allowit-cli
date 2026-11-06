//! `allowit keys export NAME`.
use crate::{ctx::Ctx, error::Result, settings::keys};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit keys export agent > agent.key
  allowit keys export agent | pass insert -m allowit/agent";

/// Print a private key in the rail's signer format
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Key name
    pub name: String,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let key = cx.key(&args.name)?;
    let dir = cx.dir()?;
    let secret = keys::read_secret(&dir, &key)?;
    cx.out.raw = true;
    cx.out.line(secret.trim());
    Ok(0)
}
