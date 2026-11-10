//! `allowit policy check FILE [--compile]`.
use crate::{ctx::Ctx, error::Result};
use std::path::PathBuf;
pub(crate) const EXAMPLES: &str = "Examples:\n  allowit policy check policy.rs\n  allowit policy check policy.rs --compile\n  cat policy.rs | allowit policy check -";
/// Validate the structure and allowed operations of portable Rust source
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Source file; - reads stdin
    pub file: PathBuf,
    /// Also type-check the source against the pinned Core host library
    #[arg(long)]
    pub compile: bool,
}
pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let source = super::read(&args.file)?;
    let shape = allowit_sdk::authoring::check(&source)?;
    if args.compile {
        allowit_sdk::authoring::compile(&source, &allowit_sdk::authoring::HostTools::default())?;
    }
    cx.out.value(&serde_json::json!({"sourceHash":allowit_sdk::digest(source.as_bytes()),"prefix":shape.prefix,"canonicalOnly":shape.canonical_only,"writesState":shape.writes_state,"compiled":args.compile}));
    Ok(0)
}
