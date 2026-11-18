//! Import an explicit public export through shared native verification.
use crate::{
    ctx::{Ctx, read_input},
    error::{Error, Result},
    presign,
};
use std::path::PathBuf;
pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit wallet import --policy POLICY_ID --server-key PUBLIC_KEY
  allowit wallet import profile.json";

/// Verify and cache a public profile without changing the selected agent key
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// The public profile file that the owner exported
    #[arg(required_unless_present = "policy", conflicts_with = "policy")]
    pub profile_file: Option<PathBuf>,
    /// Read the profile of this policy id from the ALLOWIT_URL server, as its agent
    #[arg(long, value_name = "POLICY_ID", allow_hyphen_values = true)]
    pub policy: Option<String>,
    /// The server key that `allowit POLICY_ID help` printed: the import refuses another
    #[arg(long, value_name = "PUBLIC_KEY", requires = "policy")]
    pub server_key: Option<String>,
}
pub(crate) fn run(a: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let rpc = cx
        .rpc_url(rail)?
        .ok_or_else(|| Error::config("Select an independent RPC endpoint to verify this import"))?;
    // A profile from the server takes the same checks as a file: the pinned
    // server key and the independent endpoint decide, not the transport.
    let (bytes, pin) = match (&a.profile_file, &a.policy) {
        (Some(file), _) => (read_input(file)?, None),
        (None, Some(id)) => {
            let read = presign::policy_profile(cx, rail, id, a.server_key.as_deref())?;
            (read.profile, read.pin)
        }
        (None, None) => return Err(Error::usage("name a profile file or --policy POLICY_ID")),
    };
    let authority = match &pin {
        Some(pin) => pin.authority.clone(),
        None => cx.server_authority(rail)?,
    };
    let cache = allowit_sdk::profiles::Cache::open(&cx.dir()?)?;
    let p = cache.import(&authority, &rpc, &bytes)?;
    // The policy's server key is pinned once the import stands: a later answer of another key
    // is refused before the agent signs.
    if let Some(pin) = &pin {
        pin.save(cx, rail)?;
    }
    // A profile that the agent asked for by its policy id becomes the selected version of its
    // wallet for the selected agent key. An older cached version of the same wallet, such as
    // the one before a value change, then cannot serve the next request.
    if a.policy.is_some()
        && let Ok(agent) = cx.agent(rail)
        && let Some(wallet) = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|profile| profile["wallet"].as_str().map(str::to_owned))
    {
        let _ = cache.select(&allowit_sdk::profiles::Selector {
            network: rail.id.into(),
            wallet: Some(wallet),
            agent: Some(agent.address),
            version: Some(p.version.clone()),
            ..Default::default()
        });
    }
    let mut shown = p.inspect()?;
    if let Some(pin) = &pin
        && pin.new
    {
        shown["pinnedServerKey"] = pin.authority.public_key().into();
    }
    cx.out.value(&shown);
    Ok(0)
}
