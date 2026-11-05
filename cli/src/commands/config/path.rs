//! `allowit config path`.
use crate::{ctx::Ctx, error::Result, settings::Settings};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit config path
  $EDITOR \"$(allowit config path)\"";

/// Print the path of config.toml
#[derive(clap::Args, Debug)]
pub(crate) struct Args {}

pub(crate) fn run(_: Args, cx: &mut Ctx) -> Result<i32> {
    let dir = cx.dir_path()?;
    cx.out.line(Settings::path(&dir).to_string_lossy());
    Ok(0)
}
