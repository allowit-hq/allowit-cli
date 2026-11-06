//! Hidden `allowit wallet configure`: the classic-account owner setup link of
//! earlier releases; availability follows the selected adapter.
use crate::{ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit --rail __ALLOWIT_EXAMPLE_RAIL__ wallet configure --owner OWNER --agent AGENT --engine SERVER --request-id setup-0001";

/// Print the owner signing link for account setup
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// --owner OWNER --agent AGENT --engine SERVER --request-id ID [--fee-per-operation N]
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "FLAGS"
    )]
    pub args: Vec<String>,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let vendor = cx.vendor()?;
    vendor.legacy(cx, "wallet configure", &args.args)
}
