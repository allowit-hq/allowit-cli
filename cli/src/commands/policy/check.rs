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
    if args.compile {
        let tools = allowit_sdk::authoring::HostTools::default();
        if server_profile {
            allowit_sdk::authoring::compile_server(&source, &tools)?;
        } else {
            allowit_sdk::authoring::compile(&source, &tools)?;
        }
    }
    cx.out.value(&serde_json::json!({"sourceHash":allowit_sdk::digest(source.as_bytes()),"profile":if server_profile {"server"} else {"portable"},"placement":if server_profile || !functions.is_empty() {"server"} else {"contract"},"serverFunctions":functions,"prefix":shape.prefix,"canonicalOnly":shape.canonical_only,"writesState":shape.writes_state,"compiled":args.compile}));
    Ok(0)
}
