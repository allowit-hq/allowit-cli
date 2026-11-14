//! `allowit config show`.
use crate::{config::env_value, ctx::Ctx, error::Result};
use serde_json::json;

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit config show
  allowit config show --json";

/// Show the config file, current rail and engine URL
#[derive(clap::Args, Debug)]
pub(crate) struct Args {}

pub(crate) fn run(_: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail_opt()?.map(|r| r.id);
    let rail_source = if cx.global.rail.is_some() {
        "flag or ALLOWIT_RAIL"
    } else if rail.is_some() {
        "config.toml"
    } else {
        "unset"
    };
    let settings = cx.settings()?;
    let path = crate::settings::Settings::path(&settings.dir);
    let config = serde_json::to_value(&settings.config).unwrap_or_default();
    // An unset variable means the production server.
    let implicit = env_value("ALLOWIT_URL").trim().is_empty();
    let engine_url = json!(cx.server_url()?);
    let engine_url_source = if implicit { "default" } else { "ALLOWIT_URL" };
    let value = json!({
        "path": path.to_string_lossy(),
        "rail": rail,
        "railSource": rail_source,
        "engineUrl": engine_url,
        "engineUrlSource": engine_url_source,
        "config": config,
    });
    cx.out.value(&value);
    Ok(0)
}
