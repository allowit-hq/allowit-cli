//! Select the local signing key for the current rail.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    settings::keys,
};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit keys use agent";

/// Select an existing private key for requests on the current rail
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Name of the local private key
    pub name: String,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let key = cx.key(&args.name)?;
    key.check_rail(rail.id)?;
    let path = keys::signer_path(&cx.dir()?, &key)?;
    let public = crate::wallet::inspect_wallet(rail.id, &path)?;
    if key.address != public.address
        || key.public_key.as_deref() != Some(public.public_key.as_str())
        || key.curve != public.key_type
    {
        return Err(Error::config(
            "the key metadata differs from the private signer",
        ));
    }
    cx.settings()?.update(|config| {
        config.rail_mut(rail.id).agent = Some(key.name.clone());
        Ok(())
    })?;
    cx.out.value(&super::view(&key));
    Ok(0)
}
