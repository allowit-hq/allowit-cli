//! `allowit rail show [RAIL]`.
use crate::{ctx::Ctx, error::Result, routing, vendors};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit rail show
  allowit rail show __ALLOWIT_EXAMPLE_RAIL__ --json";

/// Show a rail: its identity, what works on it, and its local settings
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Rail as CAIP-2 or alias [default: the current rail]
    #[arg(value_name = "RAIL")]
    pub name: Option<String>,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let info = match &args.name {
        Some(r) => routing::info(r)?,
        None => cx.rail()?,
    };
    let vendor = vendors::rail(info);
    let caps = vendor.caps();
    let settings = cx.settings()?.rail(info.id);
    let tokens: Vec<_> = vendor
        .tokens()
        .iter()
        .map(|t| {
            serde_json::json!({
                "code": t.code, "decimals": t.decimals, "native": t.native, "issuer": t.issuer,
            })
        })
        .collect();
    let unavailable: Vec<&str> = vendors::VERBS
        .iter()
        .copied()
        .filter(|v| !caps.verbs.contains(v))
        .collect();
    let value = serde_json::json!({
        "rail": info.id,
        "aliases": info.aliases,
        "family": info.family.name(),
        "testnet": info.testnet,
        "curve": caps.curve,
        "ops": caps.ops,
        "available": caps.verbs,
        "notAvailable": unavailable,
        "tokens": tokens,
        "wallet": settings.wallet,
        "agent": settings.agent,
    });
    if cx.out.json {
        cx.out.json_value(&value);
        return Ok(0);
    }
    let list = |v: &[&str]| {
        if v.is_empty() {
            "-".to_owned()
        } else {
            v.join(", ")
        }
    };
    let codes: Vec<&str> = vendor.tokens().iter().map(|t| t.code).collect();
    // The same keys as the JSON form, in its order; lists print joined.
    let rows = vec![
        vec!["rail".to_owned(), info.id.into()],
        vec!["aliases".into(), info.aliases.join(", ")],
        vec!["family".into(), info.family.name().into()],
        vec!["testnet".into(), info.testnet.to_string()],
        vec!["curve".into(), caps.curve.into()],
        vec!["ops".into(), list(caps.ops)],
        vec!["available".into(), list(caps.verbs)],
        vec!["notAvailable".into(), list(&unavailable)],
        vec!["tokens".into(), list(&codes)],
        vec![
            "wallet".into(),
            settings.wallet.unwrap_or_else(|| "-".into()),
        ],
        vec!["agent".into(), settings.agent.unwrap_or_else(|| "-".into())],
    ];
    cx.out.table(&rows);
    Ok(0)
}
