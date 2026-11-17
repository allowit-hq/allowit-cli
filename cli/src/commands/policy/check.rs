//! `allowit policy check FILE [--kind KIND] [--compile]`.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    kinds::{self, WalletKind},
};
use std::path::PathBuf;
pub(crate) const EXAMPLES: &str = "Examples:\n  allowit policy check policy.rs\n  allowit policy check policy.rs --compile\n  allowit policy check policy.rs --kind native\n  cat policy.rs | allowit policy check -";
/// Validate policy source with the profile of the place that evaluates it
///
/// A policy that calls a server function, and every policy of a keypair wallet (--kind native),
/// is evaluated by the server: it is checked with the server profile, as the server checks it.
/// Every other policy is compiled by a wallet contract: it is checked with the portable profile.
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
    let server = allowit_sdk::authoring::check_server(&source);
    // The server functions that the source calls. No contract can run one.
    let functions = server
        .as_ref()
        .map(|checked| checked.functions.clone())
        .unwrap_or_default();
    let on_server = match args.kind {
        Some(WalletKind::Custom) if !functions.is_empty() => {
            return Err(Error::usage(format!(
                "A Custom wallet compiles its whole source on chain, and this policy calls {}, which only the server can run. Choose the Standard wallet: its contract runs the canonical rules, and the server runs the rest.",
                functions.join(", ")
            )));
        }
        // The server enforces the whole policy of a keypair wallet.
        Some(WalletKind::Native) => true,
        Some(WalletKind::Custom) => false,
        // A Simple or Standard wallet, or no kind: the server evaluates a policy that calls a
        // server function, and a contract compiles every other one.
        _ => !functions.is_empty(),
    };
    let shape = if on_server {
        server?.shape
    } else {
        allowit_sdk::authoring::check(&source).map_err(|mut error| {
            // The source is admitted where the server evaluates it: say so.
            if server.is_ok() {
                error.message.push_str(
                    ". A policy that the server evaluates may use it: a keypair wallet's policy (check it with --kind native), or a policy that calls a server function",
                );
            }
            error
        })?
    };
    if args.compile {
        let tools = allowit_sdk::authoring::HostTools::default();
        if on_server {
            allowit_sdk::authoring::compile_server(&source, &tools)?;
        } else {
            allowit_sdk::authoring::compile(&source, &tools)?;
        }
    }
    cx.out.value(&serde_json::json!({"sourceHash":allowit_sdk::digest(source.as_bytes()),"placement":if on_server {"server"} else {"contract"},"serverFunctions":functions,"prefix":shape.prefix,"canonicalOnly":shape.canonical_only,"writesState":shape.writes_state,"compiled":args.compile}));
    Ok(0)
}
