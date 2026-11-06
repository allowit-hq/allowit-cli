//! Terminal binding for shared account setup.
use crate::{ctx::Ctx, error::Result, owner};
pub(crate) const EXAMPLES: &str = "Examples:\n  allowit --rpc-url URL wallet setup --owner-key owner --agent-key agent --server ADDRESS --request-id setup-0001 --yes";
/// Configure a fresh funded account with distinct owner, agent and server roles
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    #[arg(long)]
    pub owner_key: String,
    #[arg(long)]
    pub agent_key: String,
    #[arg(long)]
    pub server: String,
    #[arg(long)]
    pub request_id: String,
    /// Fee per operation in the selected rail's base units
    #[arg(long, default_value = "100")]
    pub fee_per_operation: String,
}
pub(crate) fn run(a: Args, cx: &mut Ctx) -> Result<i32> {
    let local = owner::load(cx, &a.owner_key)?;
    let agent = cx.key(&a.agent_key)?;
    agent.check_rail(cx.rail()?.id)?;
    cx.confirm("Set the fresh owner's account roles and signing thresholds")?;
    cx.out.value(&allowit_sdk_engine::owner::setup(
        &local.context("http://localhost"),
        allowit_sdk_engine::owner::Setup {
            agent: &agent.address,
            server: &a.server,
            request_id: &a.request_id,
            fee_per_operation: &a.fee_per_operation,
        },
    )?);
    Ok(0)
}
