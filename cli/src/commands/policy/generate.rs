//! Terminal bindings for shared server-backed policy generation.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    owner,
};
use allowit_sdk::generation::{self, GenerateRequest, Target};

pub(crate) const EXAMPLES: &str = "Examples:\n  allowit policy generate --owner-key owner --origin http://localhost:5173 --kind common --new-wallet 'Pay only my approved recipient'";

/// The policy kind an owner asks for. The server advertises one wallet
/// profile per custody: Simple and Common share the policy-wallet profile, and
/// the server chooses between them from the rules of the draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Kind {
    /// Rules on the owner's own account; no wallet contract
    Native,
    /// Policy wallet with rules that the server checks
    Simple,
    /// Policy wallet with canonical rules that the wallet also enforces
    Common,
    /// Policy wallet that runs the complete compiled policy
    Custom,
}
impl Kind {
    /// The server's wallet profile for this kind.
    fn profile(self) -> &'static str {
        match self {
            Self::Native => "allowit-native-v1",
            Self::Simple | Self::Common => "allowit-policy-wallet-v1",
            Self::Custom => "allowit-custom-wallet-v1",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Simple => "simple",
            Self::Common => "common",
            Self::Custom => "custom",
        }
    }
}

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
    /// Policy kind; the server chooses Simple or Common from the draft's rules
    #[arg(long, value_enum, value_name = "KIND")]
    pub kind: Kind,
    /// Explicitly create a draft from an advertised wallet template
    #[arg(long)]
    pub new_wallet: bool,
    /// Existing owner installation ID (64 lowercase hexadecimal characters)
    #[arg(long, value_name = "ID")]
    pub installation: Option<String>,
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
        profile: args.kind.profile().into(),
        target: if args.new_wallet {
            Target::NewWallet
        } else {
            Target::Installation(
                args.installation
                    .ok_or_else(|| Error::usage("Select --new-wallet or --installation"))?,
            )
        },
    };
    // Input checks come before any key or network access.
    request.validate()?;
    let signer = owner::signer(cx, &args.owner_key)?;
    let draft = generation::generate(
        &crate::config::env_value("ALLOWIT_URL"),
        &args.origin,
        &signer,
        &request,
        owner::clock,
    )?;
    // Simple and Common share one profile, so report a different server choice.
    if let Some(tier) = draft_tier(&draft.document)
        && matches!(args.kind, Kind::Simple | Kind::Common)
        && tier != args.kind.name()
    {
        cx.out.warning(format!(
            "the server generated a {tier} policy from the draft's rules, not {}",
            args.kind.name()
        ));
    }
    let code = if draft.unresolved.is_empty() { 0 } else { 11 };
    cx.out.value(&draft.document);
    Ok(code)
}

/// The tier that the server bound to a generated draft, when it names one.
fn draft_tier(document: &serde_json::Value) -> Option<&str> {
    document
        .pointer("/policy/runtime/instanceBinding/tier")
        .or_else(|| document.pointer("/runtime/instanceBinding/tier"))
        .and_then(serde_json::Value::as_str)
}
