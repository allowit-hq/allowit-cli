//! Owner key custody and terminal bindings for shared server-backed policy generation.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
};
use allowit_sdk_engine::generation::{self, GenerateRequest, Target};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const EXAMPLES: &str = "Examples:\n  allowit policy generate --owner-key owner --origin http://localhost:5173 --profile PROFILE --new-wallet 'Pay only my approved recipient'";

/// Generate a portable draft through the owner-authenticated server; does not deploy or spend
#[derive(clap::Args, Debug)]
#[command(group(clap::ArgGroup::new("generation_target").args(["new_wallet","installation"]).required(true)))]
pub(crate) struct Args {
    /// Complete owner instructions, preserved exactly
    pub prompt: String,
    /// Named local owner key; independent of the selected agent key
    #[arg(long, value_name = "NAME")]
    pub owner_key: String,
    /// Configured sign-in Origin, which can differ from ALLOWIT_URL
    #[arg(long, env = "ALLOWIT_OWNER_ORIGIN", hide_env_values = true)]
    pub origin: String,
    /// Exact wallet profile advertised by the server
    #[arg(long)]
    pub profile: String,
    /// Explicitly create a draft from an advertised contract-wallet template
    #[arg(long)]
    pub new_wallet: bool,
    /// Existing owner installation ID (64 lowercase hexadecimal characters)
    #[arg(long, value_name = "ID")]
    pub installation: Option<String>,
}
fn clock() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| Error::config("System clock precedes the Unix epoch"))
}
pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    if cx.global.wallet_flag {
        return Err(Error::usage(
            "Generation selects --new-wallet or --installation; omit --wallet",
        ));
    }
    let rail = cx.rail()?;
    let request = GenerateRequest {
        prompt: args.prompt,
        network: rail.id.into(),
        profile: args.profile,
        target: if args.new_wallet {
            Target::NewWallet
        } else {
            Target::Installation(
                args.installation
                    .ok_or_else(|| Error::usage("Select --new-wallet or --installation"))?,
            )
        },
    };
    request.validate()?;
    let key = cx.key(&args.owner_key)?;
    key.check_rail(rail.id)?;
    let signer = crate::wallet::local_signer(rail.id, &cx.signer(&key)?)?;
    if signer.address() != key.address {
        return Err(Error::config(
            "The owner key differs from its public metadata",
        ));
    }
    let draft = generation::generate(
        &crate::config::env_value("ALLOWIT_URL"),
        &args.origin,
        &signer,
        &request,
        clock,
    )?;
    let code = if draft.unresolved.is_empty() { 0 } else { 11 };
    cx.out.value(&draft.document);
    Ok(code)
}
