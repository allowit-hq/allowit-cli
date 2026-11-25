//! Terminal binding for the setup of one agent's wallet account.
use crate::{ctx::Ctx, error::Result, owner};
pub(crate) const EXAMPLES: &str = "Examples:\n  allowit --rpc-url URL wallet setup --owner-key owner --origin http://localhost:5173 --agent-key agent --server ADDRESS --request-id setup-0001 --amount 20 --yes";
/// Create the agent's own wallet account with one owner-signed transaction and register it with the server
///
/// The wallet is a new account, not the owner's account. The owner can do everything alone;
/// the agent and the server pay only together. With `--amount`, the same transaction funds the
/// wallet for the agent's payments: the owner's account pays the amount and every reserve. The server registers the wallet from the
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
    /// The amount that the setup makes available to the wallet for the agent's payments, in the
    /// installed asset (decimal). The owner's account provides it in the setup transaction, so no
    /// funding step follows. Without it the wallet has nothing to spend yet.
    #[arg(long)]
    pub amount: Option<String>,
}
pub(crate) fn run(a: Args, cx: &mut Ctx) -> Result<i32> {
    let local = owner::load(cx, &a.owner_key)?;
    let agent = cx.key(&a.agent_key)?;
    agent.check_rail(cx.rail()?.id)?;
    let context = local.context(&a.origin);
    // The owner confirms the network and the amount. The amount is the one that the setup signs:
    // the rail binds a request ID to its amount and refuses a retry with another one. The asset
    // is not named: the rail takes it from the server's template, or from the transaction that
    // a retried request signed before.
    cx.confirm(&question(context.network, a.amount.as_deref(), false))?;
    cx.out.value(&allowit_sdk::owner::setup(
        &context,
        allowit_sdk::owner::Setup {
            agent: &agent.address,
            server: &a.server,
            request_id: &a.request_id,
            fee_per_operation: &a.fee_per_operation,
            amount: a.amount.as_deref(),
        },
    )?);
    Ok(0)
}
/// The question of a setup: its network, with an amount what the setup makes available for the
/// agent's payments, and with `bind` the policy that a deployment binds. It holds on every rail:
/// the rail decides whether the amount moves into the wallet or the owner approves it, and which
/// deposits and fees the network takes, so the question names neither.
pub(crate) fn question(network: &str, amount: Option<&str>, bind: bool) -> String {
    let funding = amount
        .map(|amount| format!(" with {amount} of the wallet's asset for the agent's payments"))
        .unwrap_or_default();
    let signers = if bind {
        ", set its signers and bind this exact policy to it"
    } else {
        " and set its signers"
    };
    format!(
        "Set up this agent's wallet on {network} from the owner's account{funding}{signers}; the owner's account pays the network's deposits and fees for it"
    )
}
#[cfg(test)]
mod tests {
    use super::question;

    /// The owner confirms the network and the amount for the agent's payments, and no asset. The
    /// question states no transfer, reserve or fee float: those differ by rail.
    #[test]
    fn the_question_names_the_network_and_the_amount() {
        assert_eq!(
            question("net:test", Some("20"), false),
            "Set up this agent's wallet on net:test from the owner's account with 20 of the wallet's asset for the agent's payments and set its signers; the owner's account pays the network's deposits and fees for it"
        );
        assert_eq!(
            question("net:test", None, false),
            "Set up this agent's wallet on net:test from the owner's account and set its signers; the owner's account pays the network's deposits and fees for it"
        );
        assert_eq!(
            question("net:test", None, true),
            "Set up this agent's wallet on net:test from the owner's account, set its signers and bind this exact policy to it; the owner's account pays the network's deposits and fees for it"
        );
    }
}
