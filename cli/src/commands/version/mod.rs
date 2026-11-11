//! `allowit version`: the release and the git revision that built the binary.
use crate::{REVISION, VERSION, VERSION_LINE, ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit version
  allowit --version";

/// Print the version and the git revision of the build
#[derive(clap::Args, Debug)]
pub(crate) struct Args {}

pub(crate) fn run(_: Args, cx: &mut Ctx) -> Result<i32> {
    if cx.out.json {
        cx.out
            .json_value(&serde_json::json!({"version": VERSION, "revision": REVISION}));
    } else {
        cx.out.line(format!("allowit {VERSION_LINE}"));
    }
    Ok(0)
}
