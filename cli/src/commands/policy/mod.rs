//! Portable source validation, generation, owner workflows and installed-policy inspection.
mod check;
mod generate;
mod kinds;
mod owner;
mod show;
use crate::{ctx::Ctx, error::Result};
use clap::Subcommand;
pub(crate) use generate::EXAMPLES as GENERATE_EXAMPLES;
pub(crate) const EXAMPLES: &str = "Examples:\n  allowit policy kinds\n  allowit policy check policy.rs\n  allowit policy check policy.rs --compile\n  allowit policy show --source";
/// Generate and validate portable policies or inspect an installed policy
#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Read original owner transaction receipts
    OwnerStatus(owner::Status),
    /// Deploy a reviewed policy and executable
    #[command(before_help=owner::EXAMPLES)]
    Deploy(owner::Deploy),
    /// Fund the selected policy wallet
    Fund(owner::Fund),
    /// Approve exact policy source and parameters
    Activate(owner::Activate),
    /// Verify and cache the active public profile
    Export(owner::Export),
    #[command(before_help=kinds::EXAMPLES)]
    Kinds(kinds::Args),
    #[command(before_help=generate::EXAMPLES)]
    Generate(generate::Args),
    #[command(before_help=show::EXAMPLES)]
    Show(show::Args),
    #[command(before_help=check::EXAMPLES)]
    Check(check::Args),
}
pub(crate) fn run(command: Command, cx: &mut Ctx) -> Result<i32> {
    match command {
        Command::OwnerStatus(a) => owner::status(a, cx),
        Command::Deploy(a) => owner::deploy(a, cx),
        Command::Fund(a) => owner::fund(a, cx),
        Command::Activate(a) => owner::activate(a, cx),
        Command::Export(a) => owner::export(a, cx),
        Command::Kinds(a) => kinds::run(a, cx),
        Command::Generate(a) => generate::run(a, cx),
        Command::Show(a) => show::run(a, cx),
        Command::Check(a) => check::run(a, cx),
    }
}
pub(crate) fn is_local(command: &Command) -> bool {
    matches!(command, Command::Check(_))
}
fn read(path: &std::path::Path) -> Result<String> {
    String::from_utf8(crate::ctx::read_input(path)?)
        .map_err(|_| crate::error::Error::usage("The policy source must be UTF-8"))
}
