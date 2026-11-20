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
    /// The amount that the setup moves into the wallet for the agent's payments, in the installed
    /// asset (decimal). The owner's account pays it in the setup transaction, so no funding step
    /// follows. Without it the wallet holds its reserve and a fee float.
    #[arg(long)]
    pub amount: Option<String>,
}
pub(crate) fn run(a: Args, cx: &mut Ctx) -> Result<i32> {
    let local = owner::load(cx, &a.owner_key)?;
    let agent = cx.key(&a.agent_key)?;
    agent.check_rail(cx.rail()?.id)?;
    let context = local.context(&a.origin);
    // The owner confirms the network and, with an amount, the amount in the asset that the rail
    // reads from the server's template for a new wallet. The read signs the owner in, so it runs
    // only for a question that the owner sees.
    let asset = match a.amount {
        Some(_) if cx.prompts() => Some(allowit_sdk::owner::native_wallet_asset(&context)?),
        _ => None,
    };
    cx.confirm(&question(
        context.network,
        a.amount.as_deref(),
        asset.as_ref(),
    ))?;
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
/// The question of a setup: its network and, with an amount and the asset of the new wallet, what
/// the setup moves into the wallet.
fn question(
    network: &str,
    amount: Option<&str>,
    asset: Option<&allowit_sdk::owner::NativeWalletAsset>,
) -> String {
    match (amount, asset) {
        (Some(amount), Some(asset)) => {
            let asset = match &asset.issuer {
                Some(issuer) => format!("{} issued by {issuer}", asset.symbol),
                None => asset.symbol.clone(),
            };
            format!(
                "Create this agent's wallet account on {network} from the owner's account, fund it with {amount} {asset}, and set its signers"
            )
        }
        _ => format!(
            "Create this agent's wallet account on {network} from the owner's account and set its signers"
        ),
    }
}
#[cfg(test)]
mod tests {
    use super::question;
    use allowit_sdk::owner::NativeWalletAsset;

    /// The owner confirms the network, the amount and the asset that the setup moves.
    #[test]
    fn the_question_names_the_network_the_amount_and_the_asset() {
        let asset = |symbol: &str, issuer: Option<&str>| NativeWalletAsset {
            symbol: symbol.into(),
            issuer: issuer.map(str::to_owned),
        };
        assert_eq!(
            question("net:test", Some("20"), Some(&asset("COIN", None))),
            "Create this agent's wallet account on net:test from the owner's account, fund it with 20 COIN, and set its signers"
        );
        assert_eq!(
            question(
                "net:test",
                Some("2.5"),
                Some(&asset("TOKEN", Some("ISSUER")))
            ),
            "Create this agent's wallet account on net:test from the owner's account, fund it with 2.5 TOKEN issued by ISSUER, and set its signers"
        );
        assert_eq!(
            question("net:test", None, None),
            "Create this agent's wallet account on net:test from the owner's account and set its signers"
        );
    }
}
