//! `allowit config`: the local configuration.
mod path;
mod show;

use crate::{ctx::Ctx, error::Result};
use clap::Subcommand;

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit config show
  allowit config path";

/// Show the configuration and where it lives
#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    #[command(before_help = show::EXAMPLES)]
    Show(show::Args),
    #[command(before_help = path::EXAMPLES)]
    Path(path::Args),
}

pub(crate) fn run(command: Command, cx: &mut Ctx) -> Result<i32> {
    match command {
        Command::Show(a) => show::run(a, cx),
        Command::Path(a) => path::run(a, cx),
    }
}
