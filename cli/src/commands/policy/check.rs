//! `allowit policy check FILE [--kind KIND] [--compile]`.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    kinds::{self, WalletKind},
};
use std::path::PathBuf;
pub(crate) const EXAMPLES: &str = "Examples:\n  allowit policy check policy.rs\n  allowit policy check policy.rs --compile\n  allowit policy check policy.rs --kind native\n  cat policy.rs | allowit policy check -";
/// Validate policy source with the profile of the wallet kind that holds it
///
/// The server enforces the whole policy of a keypair wallet (--kind native) and keeps its
/// params: that policy is checked with the server profile, which admits the server functions
/// and the text of an owner param, read with as_str(). A contract wallet holds its source on
/// chain: that source is checked with the portable profile, also when it calls a server
/// function, which the server evaluates before it co-signs. Without --kind, a policy that only
/// the server profile admits is checked as a keypair wallet's policy.
///
/// A decision question that the source states as a literal and that asks an amount, an address,
/// a price or a date gets a warning: a rule checks such a detail exactly.
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Source file; - reads stdin
    pub file: PathBuf,
    // The command tree replaces this help with the SDK's kinds.
    /// Wallet kind that the policy is for
    #[arg(long, value_name = "KIND", value_parser = kinds::parse)]
    pub kind: Option<WalletKind>,
    /// Also type-check the source against the pinned Core host library
    #[arg(long)]
    pub compile: bool,
}
pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let source = super::read(&args.file)?;
    // The server's gate checks every policy with the server profile, whatever wallet holds
    // it. So that profile comes first: a policy that it refuses reaches no wallet. Its result
    // names the server functions that the source calls, as Core's gate reads them. No
    // contract can run one: the server evaluates such a policy before it co-signs.
    let server = allowit_sdk::authoring::check_server(&source)?;
    let portable = allowit_sdk::authoring::check(&source);
    let functions = server.functions;
    let server_profile = match args.kind {
        Some(WalletKind::Custom) if !functions.is_empty() => {
            return Err(Error::usage(format!(
                "A Custom wallet compiles its whole source on chain, and this policy calls {}, which only the server can run. Choose the Standard wallet: its contract runs the canonical rules, and the server runs the rest.",
                functions.join(", ")
            )));
        }
        // The server enforces the whole policy of a keypair wallet.
        Some(WalletKind::Native) => true,
        // A contract wallet holds its source: its rail also checks it with the portable
        // profile.
        Some(_) => false,
        // No kind: a policy that the portable profile refuses is a keypair wallet's policy.
        None => portable.is_err(),
    };
    let shape = if server_profile {
        server.shape
    } else {
        portable.map_err(|mut error| {
            // The server profile admits the source: say which wallet holds such a policy.
            error.message.push_str(
                ". Only a keypair wallet's policy may use it (--kind native): the server keeps that wallet's params. A contract wallet's policy states its text in the source",
            );
            error
        })?
    };
    // The kinds take what the server binds to them. A keypair wallet takes any policy: the
    // server evaluates all of it. A Simple or Standard wallet takes the canonical rules, or a
    // policy that calls a server function, which the server evaluates before it co-signs:
    // other custom code needs a contract that runs it, the Custom wallet.
    if let Some(kind @ (WalletKind::Simple | WalletKind::Standard)) = args.kind
        && functions.is_empty()
        && !shape.canonical_only
    {
        return Err(Error::usage(format!(
            "The {} wallet takes the canonical rules, or a policy that calls a server function, and this policy has other custom code. Choose the Native wallet, where the server evaluates the whole policy, or the Custom wallet, whose contract runs it.",
            kind.id()
        )));
    }
    if args.compile {
        let tools = allowit_sdk::authoring::HostTools::default();
        if server_profile {
            allowit_sdk::authoring::compile_server(&source, &tools)?;
        } else {
            allowit_sdk::authoring::compile(&source, &tools)?;
        }
    }
    // A question that asks a detail that a rule checks exactly leaves the result unchanged.
    for warning in super::details::warnings(&source) {
        cx.out.warning(warning);
    }
    cx.out.value(&serde_json::json!({"sourceHash":allowit_sdk::digest(source.as_bytes()),"profile":if server_profile {"server"} else {"portable"},"placement":if server_profile || !functions.is_empty() {"server"} else {"contract"},"serverFunctions":functions,"prefix":shape.prefix,"canonicalOnly":shape.canonical_only,"writesState":shape.writes_state,"compiled":args.compile}));
    Ok(0)
}
