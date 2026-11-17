//! `allowit policy check FILE [--kind KIND] [--compile]`.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    kinds::{self, WalletKind},
};
use std::path::PathBuf;
pub(crate) const EXAMPLES: &str = "Examples:\n  allowit policy check policy.rs\n  allowit policy check policy.rs --compile\n  allowit policy check policy.rs --kind native\n  allowit policy check policy.rs --kind custom --onchain\n  cat policy.rs | allowit policy check -";
/// Validate policy source with the profile of the wallet kind that holds it
///
/// The server enforces the whole policy of a keypair wallet (--kind native) and keeps its
/// params: that policy is checked with the server profile, which admits the server functions
/// and the text of an owner param, read with as_str(). A Simple or Standard wallet holds its
/// source on chain: that source is checked with the portable profile, also when it calls a
/// server function, which the server evaluates before it co-signs. Without --kind, a policy
/// that only the server profile admits is checked as a keypair wallet's policy.
///
/// A Custom wallet (--kind custom) takes one source too, checked with the server profile. The
/// server evaluates all of it before it co-signs. The wallet contract compiles the on-chain
/// part of the source: the steps that the chain can check by itself. The steps that depend on
/// a server function or on stored state leave, and the server's signature enforces them. The
/// result states the on-chain part, each check that only the server enforces and the figures
/// that the wallet keeps. A Custom source that writes policy state is refused: the wallet
/// keeps no state of its own. --onchain prints the on-chain part itself.
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
    /// Print only the on-chain part of the source: what the contract of a Custom wallet compiles
    #[arg(long)]
    pub onchain: bool,
}

/// Why a check runs only on the server, in the owner's words.
fn cause(code: &str) -> &'static str {
    match code {
        "server_call" => "it calls a server function",
        "server_value" => "it reads a value from a server function",
        "server_branch" => "it is in a branch that depends on a server function",
        "after_server_exit" => "it comes after an early approval that depends on a server function",
        "stored_state" => {
            "it reads stored state, which only the server reads for a contract wallet"
        }
        _ => "it is in a statement that cannot be split",
    }
}

/// What a result states about the on-chain part of a Custom wallet's source: the hash of the
/// derived source, the lines of the owner's source that left, the checks that stay, each
/// check that only the server enforces, and the figures that the wallet keeps for the kept
/// rules, its only state.
pub(crate) fn onchain_view(part: &allowit_sdk::authoring::Derived) -> serde_json::Value {
    serde_json::json!({
        "sourceHash": allowit_sdk::digest(part.source.as_bytes()),
        "left": part.left,
        "kept": part.kept,
        "figures": part.figures.iter().map(|figure| serde_json::json!({
            "key": figure.key,
            "daily": figure.daily,
        })).collect::<Vec<_>>(),
        "serverOnly": part.server_only.iter().map(|check| serde_json::json!({
            "line": check.line,
            "check": check.check,
            "cause": check.cause.code(),
            "movable": check.movable(),
        })).collect::<Vec<_>>(),
    })
}

/// Warn the owner about an on-chain part that checks nothing, and about each rule that runs
/// only on the server because of where it stands in `source`.
pub(crate) fn warn(cx: &mut Ctx, part: &allowit_sdk::authoring::Derived, source: &str) {
    if part.source != source && part.kept.is_empty() {
        cx.out.warning("The on-chain part of this policy checks nothing: the wallet contract enforces the two signatures and its own effect checks, and the server enforces the policy.");
    }
    for check in part.server_only.iter().filter(|check| check.movable()) {
        cx.out.warning(format!(
            "Line {}: {} runs only on the server, because {}. Move it out and the wallet contract enforces it too.",
            check.line,
            check.check,
            cause(check.cause.code())
        ));
    }
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
    let kind = match (args.kind, args.onchain) {
        (None, true) => Some(WalletKind::Custom),
        (Some(kind), true) if kind != WalletKind::Custom => {
            return Err(Error::usage(
                "--onchain prints what the contract of a Custom wallet compiles; use it with --kind custom",
            ));
        }
        (kind, _) => kind,
    };
    // The contract of a Custom wallet compiles the on-chain part of its one source. Core
    // derives it; a source whose part cannot be derived reaches no Custom wallet.
    let onchain = (kind == Some(WalletKind::Custom))
        .then(|| allowit_sdk::authoring::onchain_part(&source))
        .transpose()?;
    let server_profile = match kind {
        // The server evaluates the whole policy of a keypair wallet and of a Custom wallet.
        Some(WalletKind::Native | WalletKind::Custom) => true,
        // A Simple or Standard wallet holds its source: its rail also checks it with the
        // portable profile.
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
                ". Only the policy of a keypair wallet (--kind native) or of a Custom wallet (--kind custom) may use it: the server evaluates that whole policy. A Simple or Standard wallet's policy states its text in the source",
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
        // The contract compiles the on-chain part by itself.
        if let Some(part) = onchain.as_ref().filter(|part| part.source != source) {
            allowit_sdk::authoring::compile(&part.source, &tools)?;
        }
    }
    if args.onchain {
        let part = onchain.expect("the Custom kind derives the on-chain part");
        cx.out.stdout.push_str(&part.source);
        return Ok(0);
    }
    // The server evaluates a policy that the contract of its wallet does not run in full.
    let served = server_profile && kind != Some(WalletKind::Custom) || !functions.is_empty();
    let split = onchain.as_ref().is_some_and(|part| part.source != source);
    let placement = match (kind, served || split) {
        (Some(WalletKind::Custom), true) => "contract and server",
        (_, true) => "server",
        (_, false) => "contract",
    };
    let view = onchain.as_ref().map(onchain_view);
    if let Some(part) = &onchain {
        warn(cx, part, &source);
    }
    cx.out.value(&serde_json::json!({"sourceHash":allowit_sdk::digest(source.as_bytes()),"profile":if server_profile {"server"} else {"portable"},"placement":placement,"serverFunctions":functions,"prefix":shape.prefix,"canonicalOnly":shape.canonical_only,"writesState":shape.writes_state,"compiled":args.compile,"onchain":view}));
    Ok(0)
}
