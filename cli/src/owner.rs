//! Local owner selection. The shared SDK owns sessions, verification and retained workflows.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
};
use allowit_sdk_engine::{owner::Context, wallet::LocalSigner};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
pub(crate) struct LocalOwner {
    network: &'static str,
    rpc: String,
    root: PathBuf,
    signer: LocalSigner,
    backend: String,
    authority: allowit_sdk_engine::config::ServerAuthority,
}
pub(crate) fn clock() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| Error::config("System clock precedes the Unix epoch"))
}
impl LocalOwner {
    pub(crate) fn context<'a>(&'a self, origin: &'a str) -> Context<'a> {
        Context {
            network: self.network,
            backend: &self.backend,
            authority: &self.authority,
            origin,
            rpc_url: &self.rpc,
            root: &self.root,
            signer: &self.signer,
            clock: &clock,
        }
    }
}
pub(crate) fn load(cx: &mut Ctx, name: &str) -> Result<LocalOwner> {
    let rail = cx.rail()?;
    let key = cx.key(name)?;
    key.check_rail(rail.id)?;
    let signer = allowit_sdk_engine::wallet::local_signer(rail.id, &cx.signer(&key)?)?;
    if signer.address() != key.address {
        return Err(Error::config("Owner key differs from its public metadata"));
    }
    let rpc = cx.rpc_url(rail)?.filter(|v| !v.is_empty()).ok_or_else(|| {
        Error::config("Select an independent RPC endpoint with --rpc-url or the rail settings")
    })?;
    Ok(LocalOwner {
        network: rail.id,
        rpc,
        root: cx.dir()?,
        signer,
        backend: crate::config::env_value("ALLOWIT_URL"),
        authority: cx.server_authority(rail)?,
    })
}
pub(crate) fn status(cx: &mut Ctx, id: &str) -> Result<serde_json::Value> {
    let rail = cx.rail()?;
    let rpc = cx
        .rpc_url(rail)?
        .ok_or_else(|| Error::config("Select an independent RPC endpoint"))?;
    allowit_sdk_engine::owner::status(&cx.dir()?, rail.id, &rpc, id)
}
