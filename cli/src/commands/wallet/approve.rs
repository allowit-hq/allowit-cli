//! Hidden `allowit wallet approve`: the source approval link of earlier
//! releases for an installed account policy; availability follows the adapter.
use crate::{ctx::Ctx, error::Result};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit --rail __ALLOWIT_EXAMPLE_RAIL__ wallet approve --policy-file policy.json --owner OWNER --request-id approve-0001";

/// Print the owner approval link for an installed account policy
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// --policy-file FILE --owner OWNER --request-id ID
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "FLAGS"
    )]
    pub args: Vec<String>,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let vendor = cx.vendor()?;
    vendor.legacy(cx, "wallet approve", &args.args)
}
