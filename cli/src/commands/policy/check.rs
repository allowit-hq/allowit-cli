//! `allowit policy check FILE`.
use crate::{ctx::Ctx, error::Result};
use std::path::PathBuf;
pub(crate) const EXAMPLES: &str =
    "Examples:\n  allowit policy check policy.rs\n  cat policy.rs | allowit policy check -";
/// Validate the structure and allowed operations of portable Rust source
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Source file; - reads stdin
    pub file: PathBuf,
}
pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let source = super::read(&args.file)?;
    let shape = allowit_sdk_engine::authoring::check(&source)?;
    cx.out.value(&serde_json::json!({"sourceHash":allowit_sdk_engine::digest(source.as_bytes()),"prefix":shape.prefix,"canonicalOnly":shape.canonical_only,"writesState":shape.writes_state}));
    Ok(0)
}
