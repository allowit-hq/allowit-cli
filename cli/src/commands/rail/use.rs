//! `allowit rail use RAIL`.
use crate::{ctx::Ctx, error::Result, routing};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit rail use __ALLOWIT_EXAMPLE_RAIL__";

/// Choose the current rail. Each rail keeps its own wallet and agent key
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Rail as CAIP-2 or alias
    #[arg(value_name = "RAIL")]
    pub name: String,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let info = routing::info(&args.name)?;
    cx.settings()?.update(|config| {
        config.rail = Some(info.id.into());
        Ok(())
    })?;
    cx.out.value(&serde_json::json!({"rail": info.id}));
    Ok(0)
}
