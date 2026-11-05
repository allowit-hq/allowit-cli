//! Import an explicit public export through shared native verification.
use crate::{
    ctx::{Ctx, read_input},
    error::{Error, Result},
};
use std::path::PathBuf;
/// Verify and cache a public profile without changing the selected agent key
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    pub profile_file: PathBuf,
}
pub(crate) fn run(a: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let rpc = cx
        .rpc_url(rail)?
        .ok_or_else(|| Error::config("Select an independent RPC endpoint to verify this import"))?;
    let authority = cx.server_authority(rail)?;
    let bytes = read_input(&a.profile_file)?;
    let p =
        allowit_sdk_engine::profiles::Cache::open(&cx.dir()?)?.import(&authority, &rpc, &bytes)?;
    cx.out.value(&p.inspect()?);
    Ok(0)
}
