//! `allowit keys`: named local keys. Public metadata lives in `keys/NAME.toml`;
//! a private key never leaves `secrets/NAME` except through `keys export`.
mod add;
mod export;
mod fund;
mod generate;
mod import;
mod ls;
mod rm;
mod show;
mod use_key;

use crate::{ctx::Ctx, error::Result};
use clap::Subcommand;

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit keys generate agent
  allowit keys add owner --public-key PUBLIC_KEY
  allowit keys ls
  allowit keys use agent
  allowit keys show agent";

/// Manage local keys and select the signing key
#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    #[command(before_help = generate::EXAMPLES)]
    Generate(generate::Args),
    #[command(before_help = add::EXAMPLES)]
    Add(add::Args),
    #[command(before_help = import::EXAMPLES)]
    Import(import::Args),
    #[command(visible_alias = "list", before_help = ls::EXAMPLES)]
    Ls(ls::Args),
    #[command(before_help = show::EXAMPLES)]
    Show(show::Args),
    #[command(name = "use", before_help = use_key::EXAMPLES)]
    Use(use_key::Args),
    #[command(before_help = fund::EXAMPLES)]
    Fund(fund::Args),
    #[command(before_help = export::EXAMPLES)]
    Export(export::Args),
    #[command(before_help = rm::EXAMPLES)]
    Rm(rm::Args),
}

pub(crate) fn run(command: Command, cx: &mut Ctx) -> Result<i32> {
    match command {
        Command::Generate(a) => generate::run(a, cx),
        Command::Add(a) => add::run(a, cx),
        Command::Import(a) => import::run(a, cx),
        Command::Ls(a) => ls::run(a, cx),
        Command::Show(a) => show::run(a, cx),
        Command::Use(a) => use_key::run(a, cx),
        Command::Fund(a) => fund::run(a, cx),
        Command::Export(a) => export::run(a, cx),
        Command::Rm(a) => rm::run(a, cx),
    }
}

/// The public view of a key.
pub(crate) fn view(key: &crate::settings::keys::Key) -> serde_json::Value {
    key.view()
}
