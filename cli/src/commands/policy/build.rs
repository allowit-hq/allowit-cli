//! `allowit policy build FILE [-o OUT]`.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
};
use std::path::PathBuf;
pub(crate) const EXAMPLES: &str =
    "Examples:\n  allowit policy build policy.rs\n  allowit policy build policy.rs -o wallet.rs";
/// Type-check portable Rust against the pinned Core host library and emit its source
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Source file; - reads stdin
    pub file: PathBuf,
    /// Write the source to OUT instead of stdout
    #[arg(short = 'o', long = "out", value_name = "OUT")]
    pub out: Option<PathBuf>,
}
pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let source = super::read(&args.file)?;
    allowit_sdk_engine::authoring::compile(
        &source,
        &allowit_sdk_engine::authoring::HostTools::default(),
    )?;
    match args.out {
        Some(path) => std::fs::write(&path, &source)
            .map_err(|e| Error::usage(format!("{}: {e}", path.display())))?,
        None => {
            cx.out.raw = true;
            cx.out.source(&source);
        }
    }
    Ok(0)
}
