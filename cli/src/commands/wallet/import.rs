//! Import an explicit public export through shared native verification.
use crate::{
    ctx::{Ctx, read_input},
    error::{Error, Result},
};
use std::path::PathBuf;
pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit wallet import --policy POLICY_ID
  allowit wallet import profile.json";

/// Verify and cache a public profile without changing the selected agent key
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// The public profile file that the owner exported
    #[arg(required_unless_present = "policy", conflicts_with = "policy")]
    pub profile_file: Option<PathBuf>,
    /// Read the public profile of this policy id from the ALLOWIT_URL server
    #[arg(long, value_name = "POLICY_ID", allow_hyphen_values = true)]
    pub policy: Option<String>,
}
pub(crate) fn run(a: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let rpc = cx
        .rpc_url(rail)?
        .ok_or_else(|| Error::config("Select an independent RPC endpoint to verify this import"))?;
    let authority = cx.server_authority(rail)?;
    // A profile from the server takes the same checks as a file: the pinned
    // server key and the independent endpoint decide, not the transport.
    let bytes = match (&a.profile_file, &a.policy) {
        (Some(file), _) => read_input(file)?,
        (None, Some(id)) => {
            let (_, facts) = crate::commands::policy_help::fetch(cx, id)?;
            let profile = facts.profile.ok_or_else(|| {
                Error::config(format!(
                    "this policy is not active yet; run allowit {id} help"
                ))
            })?;
            // The answer must be the profile of the policy that was asked for: the checks
            // below prove the server and the chain state of a profile, not which policy it is.
            if profile["policyId"].as_str() != Some(id.as_str()) {
                return Err(Error::uncertain(
                    "The server returned the profile of another policy",
                ));
            }
            serde_json::to_vec(&profile).map_err(|_| Error::config("Cannot encode the profile"))?
        }
        (None, None) => return Err(Error::usage("name a profile file or --policy POLICY_ID")),
    };
    let p = allowit_sdk::profiles::Cache::open(&cx.dir()?)?.import(&authority, &rpc, &bytes)?;
    cx.out.value(&p.inspect()?);
    Ok(0)
}
