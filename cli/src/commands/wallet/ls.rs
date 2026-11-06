//! `allowit wallet ls`.
use crate::{ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit wallet ls
  allowit wallet ls --json";

/// List the wallets this device knows on the current rail
#[derive(clap::Args, Debug)]
pub(crate) struct Args {}

pub(crate) fn run(_: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let settings = cx.settings()?.rail(rail.id);
    if cx.out.json {
        let wallets: Vec<_> = settings
            .wallets
            .iter()
            .map(|w| serde_json::json!({"wallet": w, "default": settings.wallet.as_ref() == Some(w)}))
            .collect();
        cx.out.json_value(&serde_json::Value::Array(wallets));
        return Ok(0);
    }
    for w in &settings.wallets {
        let mark = if settings.wallet.as_ref() == Some(w) {
            "* "
        } else {
            "  "
        };
        cx.out.line(format!("{mark}{w}"));
    }
    Ok(0)
}
