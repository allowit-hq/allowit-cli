//! `allowit wallet`: policy wallets. A wallet is identified only by its
//! address; `wallet use` caches it as the rail's default.
mod import;
mod ls;
mod setup;
mod show;
mod r#use;

use crate::{ctx::Ctx, error::Result};
use clap::Subcommand;

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit wallet use WALLET
  allowit wallet show";

/// Select and inspect installed policy wallets
#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    Import(import::Args),
    #[command(before_help=setup::EXAMPLES)]
    Setup(setup::Args),
    #[command(visible_alias = "list", before_help = ls::EXAMPLES)]
    Ls(ls::Args),
    #[command(before_help = show::EXAMPLES)]
    Show(show::Args),
    #[command(before_help = r#use::EXAMPLES)]
    Use(r#use::Args),
}

/// `wallet show` for a caller inside the process, such as the `mcp` tools.
pub(crate) fn show(cx: &mut Ctx) -> Result<i32> {
    show::run(show::Args {}, cx)
}

pub(crate) fn run(command: Command, cx: &mut Ctx) -> Result<i32> {
    match command {
        Command::Import(a) => import::run(a, cx),
        Command::Setup(a) => setup::run(a, cx),
        Command::Ls(a) => ls::run(a, cx),
        Command::Show(a) => show::run(a, cx),
        Command::Use(a) => r#use::run(a, cx),
    }
}
