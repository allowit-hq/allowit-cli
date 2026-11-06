//! `allowit rail`: the CAIP-2 rail registry and the current rail.
mod ls;
mod show;
mod trust;
mod r#use;

use crate::{ctx::Ctx, error::Result};
use clap::Subcommand;

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit rail ls
  allowit rail use __ALLOWIT_EXAMPLE_RAIL__
  allowit rail show";

/// List rails, show what a rail supports and choose the current rail
#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    Trust(trust::Args),
    #[command(visible_alias = "list", before_help = ls::EXAMPLES)]
    Ls(ls::Args),
    #[command(before_help = show::EXAMPLES)]
    Show(show::Args),
    #[command(before_help = r#use::EXAMPLES)]
    Use(r#use::Args),
}

pub(crate) fn run(command: Command, cx: &mut Ctx) -> Result<i32> {
    match command {
        Command::Trust(a) => trust::run(a, cx),
        Command::Ls(a) => ls::run(a, cx),
        Command::Show(a) => show::run(a, cx),
        Command::Use(a) => r#use::run(a, cx),
    }
}
