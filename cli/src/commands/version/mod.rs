//! `allowit version`.
use crate::{VERSION, ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit version
  allowit --version";

/// Print the version
#[derive(clap::Args, Debug)]
pub(crate) struct Args {}

pub(crate) fn run(_: Args, cx: &mut Ctx) -> Result<i32> {
    if cx.out.json {
        cx.out.json_value(&serde_json::json!({"version": VERSION}));
    } else {
        cx.out.line(format!("allowit {VERSION}"));
    }
    Ok(0)
}
