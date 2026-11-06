//! Explicit public backend trust. No discovery response can install a pin.
use crate::{ctx::Ctx, error::Result};
/// Configure an independently verified public server identity for the selected rail
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    pub origin: String,
    pub public_key: String,
}
pub(crate) fn run(a: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let authority =
        allowit_sdk_engine::config::ServerAuthority::new(rail.id, &a.origin, &a.public_key)?;
    cx.settings()?.update(|config| {
        config
            .rail_mut(rail.id)
            .server_authorities
            .insert(authority.origin().into(), authority.public_key().into());
        Ok(())
    })?;
    cx.out.value(&serde_json::json!({"rail":rail.id,"origin":authority.origin(),"publicKey":authority.public_key()}));
    Ok(0)
}
