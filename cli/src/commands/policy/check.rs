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
/// Whether the text of `source` names a server function of Core as `allowit::<function>`, or
/// in a `use allowit::{..}` list. It reads the raw text, comments included, so it can say yes
/// for a source that calls none. It is used only for a source that the server profile
/// refuses, where the profile itself names no function: there a doubt refuses the policy.
fn may_call_server_function(source: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    allowit_sdk::authoring::SERVER_FUNCTIONS
        .iter()
        .any(|(namespace, name)| {
            source.match_indices(name).any(|(at, _)| {
                let (before, after) = (&source[..at], &source[at + name.len()..]);
                if before.chars().next_back().is_some_and(word)
                    || after.chars().next().is_some_and(word)
                {
                    return false;
                }
                let path = before.trim_end();
                // `allowit::name`, or `name` in the braces of `allowit::{ .. }`.
                let direct = path
                    .strip_suffix("::")
                    .is_some_and(|path| path.trim_end().ends_with(namespace));
                let listed = path.rfind('{').is_some_and(|open| {
                    !path[open..].contains('}')
                        && path[..open]
                            .trim_end()
                            .strip_suffix("::")
                            .is_some_and(|path| path.trim_end().ends_with(namespace))
                });
                direct || listed
            })
        })
}
pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let source = super::read(&args.file)?;
    let server = allowit_sdk::authoring::check_server(&source);
    let portable = allowit_sdk::authoring::check(&source);
    // The server functions that the source calls. No contract can run one: the server
    // evaluates such a policy before it co-signs.
    let functions = match &server {
        Ok(checked) => checked.functions.clone(),
        // The server profile refuses the source, so it names no function for it. A source
        // that may call a server function is then no contract's policy either: it ends here,
        // with the server profile's reason, and never passes as source for a contract.
        Err(refused) if may_call_server_function(&source) => {
            return Err(Error::usage(format!(
                "{}. This policy names a server function, which only the server can run, and the server profile refuses the policy",
                refused.message
            )));
        }
        Err(_) => Vec::new(),
    };
    let server_profile = match args.kind {
        Some(WalletKind::Custom) if !functions.is_empty() => {
            return Err(Error::usage(format!(
                "A Custom wallet compiles its whole source on chain, and this policy calls {}, which only the server can run. Choose the Standard wallet: its contract runs the canonical rules, and the server runs the rest.",
                functions.join(", ")
            )));
        }
        // The server enforces the whole policy of a keypair wallet.
        Some(WalletKind::Native) => true,
        // A contract wallet holds its source: the rail checks it with the portable profile.
        Some(_) => false,
        // No kind: a policy that only the server profile admits is a keypair wallet's policy.
        None => portable.is_err() && server.is_ok(),
    };
    let shape = if server_profile {
        server?.shape
    } else {
        portable.map_err(|mut error| {
            // The server profile admits the source: say which wallet holds such a policy.
            if server.is_ok() {
                error.message.push_str(
                    ". Only a keypair wallet's policy may use it (--kind native): the server keeps that wallet's params. A contract wallet's policy states its text in the source",
                );
            }
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
