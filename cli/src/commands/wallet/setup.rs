//! Terminal binding for the setup of one agent's wallet account.
use crate::{ctx::Ctx, error::Result, owner};
pub(crate) const EXAMPLES: &str = "Examples:\n  allowit --rpc-url URL wallet setup --owner-key owner --origin http://localhost:5173 --agent-key agent --server ADDRESS --request-id setup-0001 --yes";
/// Create the agent's own wallet account with one owner-signed transaction and register it with the server
///
/// The wallet is a new account, not the owner's account. The owner can do everything alone;
/// the agent and the server pay only together. The server registers the wallet from the
/// ledger and replies with its installation ID for `policy generate --installation`.
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    #[arg(long)]
    pub owner_key: String,
    /// Owner sign-in Origin admitted by the backend
    #[arg(long, env = "ALLOWIT_OWNER_ORIGIN", hide_env_values = true)]
    pub origin: String,
    #[arg(long)]
    pub agent_key: String,
    #[arg(long)]
    pub server: String,
    /// Stable workflow ID; resume with the same ID and arguments
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
    cx.confirm("Create this agent's wallet account from the owner's account and set its signers")?;
    cx.out.value(&allowit_sdk::owner::setup(
        &local.context(&a.origin),
        allowit_sdk::owner::Setup {
            agent: &agent.address,
            server: &a.server,
            request_id: &a.request_id,
            fee_per_operation: &a.fee_per_operation,
        },
    )?);
    Ok(0)
}
