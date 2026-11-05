//! `allowit wallet`: policy wallets. A wallet is identified only by its
//! address; `wallet use` caches it as the rail's default.
mod approve;
mod configure;
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
    #[command(hide = true, before_help = configure::EXAMPLES)]
    Configure(configure::Args),
    #[command(hide = true, before_help = approve::EXAMPLES)]
    Approve(approve::Args),
}

pub(crate) fn run(command: Command, cx: &mut Ctx) -> Result<i32> {
    match command {
        Command::Import(a) => import::run(a, cx),
        Command::Setup(a) => setup::run(a, cx),
        Command::Ls(a) => ls::run(a, cx),
        Command::Show(a) => show::run(a, cx),
        Command::Use(a) => r#use::run(a, cx),
        Command::Configure(a) => configure::run(a, cx),
        Command::Approve(a) => approve::run(a, cx),
    }
}
