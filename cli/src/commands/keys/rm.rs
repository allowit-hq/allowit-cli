//! `allowit keys rm NAME`.
use crate::{ctx::Ctx, error::Result, settings::keys};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit keys rm old-agent
  allowit keys rm old-agent --yes";

/// Remove a key and its private key file
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Key name
    pub name: String,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let key = cx.key(&args.name)?;
    cx.confirm(&format!(
        "Remove key {}{}",
        key.name,
        if key.secret {
            " and its private key"
        } else {
            ""
        }
    ))?;
    let dir = cx.dir()?;
    keys::remove(&dir, &key.name)?;
    // A rail whose agent was this key has no agent now.
    cx.settings()?.update(|config| {
        for rail in config.rails.values_mut() {
            if rail.agent.as_deref() == Some(key.name.as_str()) {
                rail.agent = None;
            }
        }
        Ok(())
    })?;
    Ok(0)
}
