//! `allowit rail ls`.
use crate::{ctx::Ctx, error::Result, routing};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit rail ls
  allowit rail ls --json";

/// List the admitted rails with their CAIP-2 identifiers and aliases
#[derive(clap::Args, Debug)]
pub(crate) struct Args {}

pub(crate) fn run(_: Args, cx: &mut Ctx) -> Result<i32> {
    let current = cx.rail_opt().ok().flatten().map(|r| r.id);
    if cx.out.json {
        let rails: Vec<_> = routing::all()
            .iter()
            .map(|r| {
                serde_json::json!({
                    "rail": r.id,
                    "aliases": r.aliases,
                    "family": r.family.name(),
                    "testnet": r.testnet,
                    "current": current == Some(r.id),
                })
            })
            .collect();
        cx.out.json_value(&serde_json::Value::Array(rails));
        return Ok(0);
    }
    let mut rows = vec![vec![
        String::new(),
        "RAIL".into(),
        "ALIAS".into(),
        "NETWORK".into(),
    ]];
    for r in routing::all() {
        rows.push(vec![
            if current == Some(r.id) { "*" } else { "" }.into(),
            r.id.into(),
            r.aliases.join(", "),
            if r.testnet { "testnet" } else { "mainnet" }.into(),
        ]);
    }
    cx.out.table(&rows);
    Ok(0)
}
