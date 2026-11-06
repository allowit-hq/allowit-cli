//! `allowit keys import NAME FILE|-`.
use crate::{
    ctx::{Ctx, read_input},
    error::{Error, Result},
    settings::keys,
};
use std::path::PathBuf;

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit keys import agent agent.key
  pass show allowit/agent | allowit keys import agent -";

/// Import a private key in the selected rail's signer format
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Key name
    pub name: String,
    /// The private key file; - reads stdin
    pub file: PathBuf,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let dir = cx.dir()?;
    keys::valid_name(&args.name)?;
    let bytes = zeroize::Zeroizing::new(read_input(&args.file)?);
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| Error::usage("the key file is not text"))?
        .trim();
    let key = keys::import_secret(&dir, &args.name, rail.id, text.as_bytes())?;
    cx.out.value(&super::view(&key));
    Ok(0)
}
