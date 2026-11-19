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
/// that writes state is checked as a Custom wallet's policy, the only kind that holds it, and
/// any other policy that only the server profile admits as a keypair wallet's policy.
///
/// A Custom wallet (--kind custom) takes one source too, checked with the server profile. The
/// server evaluates all of it before it co-signs. The wallet contract compiles the on-chain
/// part of the source: the steps that the chain can check by itself. The steps that depend on
/// a server function, read owner text with `as_str()`, or use policy state that the server
/// keeps, leave, and the server's signature enforces them. The result states the on-chain part,
/// each check that only the server enforces, the figures that the wallet keeps and each state
/// key with its keeper: the wallet contract keeps a key when a step that uses it stays and no
/// step that writes it leaves, and the server keeps every other key. --onchain prints the
/// on-chain part itself. A policy that writes state is refused for every other kind, also when
/// the kind is resolved without --kind. The result states the placement of a keypair wallet's
/// policy (the server) and of a Custom wallet's (the contract runs the on-chain part, the server
/// the part that left), and none for a Simple or Standard wallet.
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
    /// Print only the on-chain part of the source: what the contract of a Custom wallet compiles
    #[arg(long)]
    pub onchain: bool,
}

/// Why a check runs only on the server, in the owner's words.
fn cause(code: &str) -> &'static str {
    match code {
        "server_call" => "it calls a server function",
        "server_value" => "it reads a value from a server function",
        "server_branch" => "it is in a branch whose condition only the server can evaluate",
        "after_server_exit" => {
            "it comes after an early exit that only the server can evaluate, which can skip it"
        }
        "stored_state" => "it uses policy state that the server keeps",
        _ => "it is in a statement that cannot be split",
    }
}

/// What a result states about the on-chain part of a Custom wallet's source: the hash of the
/// derived source, the lines of the owner's source that left, the checks that stay, each
/// check that only the server enforces, the figures that the wallet keeps for the kept
/// statements, and each state key of the policy with its keeper: `chain` for the wallet
/// contract, `server` for the server.
pub(crate) fn onchain_view(part: &allowit_sdk::authoring::Derived) -> serde_json::Value {
    serde_json::json!({
        "sourceHash": allowit_sdk::digest(part.source.as_bytes()),
        "left": part.left,
        "kept": part.kept,
        "figures": part.figures.iter().map(|figure| serde_json::json!({
            "key": figure.key,
            "daily": figure.daily,
        })).collect::<Vec<_>>(),
        "state": part.state.iter().map(|entry| serde_json::json!({
            "key": entry.key,
            "keeper": entry.keeper.code(),
            "daily": entry.daily,
        })).collect::<Vec<_>>(),
        "serverOnly": part.server_only.iter().map(|check| serde_json::json!({
            "line": check.line,
            "check": check.check,
            "cause": check.cause.code(),
            "movable": check.movable(),
        })).collect::<Vec<_>>(),
    })
}

/// Warn the owner about an on-chain part in which no canonical rule, assertion or `return Err(...)`
/// stays (Core's `kept` names only those checks), and about each rule that runs only on the server
/// because of where it stands in `source`.
pub(crate) fn warn(cx: &mut Ctx, part: &allowit_sdk::authoring::Derived, source: &str) {
    // The server states the same warning for a part that differs from the source or keeps
    // state.
    if part.kept.is_empty() && (part.source != source || !part.state.is_empty()) {
        cx.out.warning("No canonical rule, assertion or `return Err(...)` of this policy stays in its on-chain part: the server enforces them before it co-signs, and the wallet contract runs the on-chain part, the two signatures and its own effect checks.");
    }
    for check in part.server_only.iter().filter(|check| check.movable()) {
        cx.out.warning(format!(
            "Line {}: {} runs only on the server, because {}. If it applies to every request, call it before that statement, with the values that it reads, and the wallet contract enforces it too.",
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
    // The kind that holds the policy, resolved once: every check below reads this kind, so a
    // result never depends on whether `--kind` was given.
    let kind = match (args.kind, args.onchain) {
        (None, true) => Some(WalletKind::Custom),
        (Some(kind), true) if kind != WalletKind::Custom => {
            return Err(Error::usage(
                "--onchain prints what the contract of a Custom wallet compiles; use it with --kind custom",
            ));
        }
        (Some(kind), _) => Some(kind),
        // No kind: only a Custom wallet holds a policy that keeps its own state, and a policy
        // that only the server profile admits is otherwise a keypair wallet's policy.
        (None, false) if server.shape.writes_state => Some(WalletKind::Custom),
        (None, false) if portable.is_err() => Some(WalletKind::Native),
        (None, false) => None,
    };
    // The contract of a Custom wallet compiles the on-chain part of its one source. Core
    // derives it; a source whose part cannot be derived reaches no Custom wallet.
    let onchain = (kind == Some(WalletKind::Custom))
        .then(|| allowit_sdk::authoring::onchain_part(&source))
        .transpose()?;
    // The rail's Custom wallet holds the part within its own limits, such as its storage of the
    // state keys that the contract keeps: the rail's own check refuses what its build would. The
    // rail is the selected one (`--rail`, `ALLOWIT_RAIL`), or else every rail that builds a
    // Custom wallet.
    if onchain.is_some() {
        let rails: Vec<&str> = match cx.global.rail.as_deref() {
            Some(rail) => vec![crate::routing::normalize(rail)?],
            None => allowit_sdk::routing::all()
                .iter()
                .map(|rail| rail.id)
                .collect(),
        };
        for network in rails {
            if allowit_sdk::custom_build::supported(network) {
                allowit_sdk::custom_build::check(network, &source)?;
            }
        }
    }
    // The server evaluates the whole policy of a keypair wallet and of a Custom wallet. A Simple
    // or Standard wallet holds its source: its rail also checks it with the portable profile.
    let server_profile = matches!(kind, Some(WalletKind::Native | WalletKind::Custom));
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
    // Only a Custom wallet keeps a policy's own state: its contract stores the keys of the
    // on-chain part, and the server keeps the keys of the part that only it runs. Core's gate
    // finds the state writes.
    if let Some(kind @ (WalletKind::Native | WalletKind::Simple | WalletKind::Standard)) = kind
        && shape.writes_state
    {
        return Err(Error::usage(format!(
            "The {} wallet stores no policy state, and this policy writes state with allowit::set_state. Choose the Custom wallet, or state the limit with a canonical rule, which keeps its own figure.",
            kind.id()
        )));
    }
    // The kinds take what the server binds to them. A keypair wallet takes any policy: the
    // server evaluates all of it. A Simple or Standard wallet takes the canonical rules, or a
    // policy that calls a server function, which the server evaluates before it co-signs:
    // other custom code needs a contract that runs it, the Custom wallet.
    if let Some(kind @ (WalletKind::Simple | WalletKind::Standard)) = kind
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
    // The warnings go to stderr, also when stdout carries the on-chain part.
    if let Some(part) = &onchain {
        warn(cx, part, &source);
    }
    // A question that asks a detail that a rule checks exactly leaves the result unchanged.
    for warning in super::details::warnings(&source) {
        cx.out.warning(warning);
    }
    if args.onchain {
        let part = onchain.expect("the Custom kind derives the on-chain part");
        // With `--json` the part is the `source` field of one JSON object.
        cx.out.source(&part.source);
        return Ok(0);
    }
    // Where the policy is enforced. The server evaluates the whole policy of a keypair wallet.
    // Core's derivation states it for a Custom wallet: the contract runs the on-chain part, and
    // the server evaluates the part that left. The canonical rules that a Simple or Standard
    // wallet runs on chain are those that its installation binds, which this check does not
    // read: it states no placement for them, nor for a policy whose kind it does not know.
    let placement = match (kind, &onchain) {
        (Some(WalletKind::Native), _) => Some("server"),
        (_, Some(part)) if part.source == source => Some("contract"),
        (_, Some(_)) => Some("contract and server"),
        _ => None,
    };
    let view = onchain.as_ref().map(onchain_view);
    cx.out.value(&serde_json::json!({"sourceHash":allowit_sdk::digest(source.as_bytes()),"profile":if server_profile {"server"} else {"portable"},"placement":placement,"serverFunctions":functions,"prefix":shape.prefix,"canonicalOnly":shape.canonical_only,"writesState":shape.writes_state,"compiled":args.compile,"onchain":view}));
    Ok(0)
}
