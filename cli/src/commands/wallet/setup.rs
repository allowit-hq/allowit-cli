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
    // A setup ID that the rail's journal holds resumes that setup: a transaction that was signed
    // then keeps its own asset, so the question names none and no template is read. A new setup
    // names its network and, with an amount, the asset that the rail reads from the server's
    // template for a new wallet. The read signs the owner in, so it runs only for a question
    // that the owner sees. With `--yes` no question is asked, so nothing is read.
    let question = if cx.global.yes {
        String::new()
    } else if retained(&owner::status(cx, &a.request_id)) {
        resumed(context.network, &a.request_id)
    } else {
        let asset = match a.amount {
            Some(_) if cx.prompts() => Some(allowit_sdk::owner::native_wallet_asset(&context)?),
            _ => None,
        };
        question(context.network, a.amount.as_deref(), asset.as_ref())
    };
    cx.confirm(&question)?;
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
/// Whether the rail's journal may hold a setup under the ID, from its status read. Only the
/// read's own answer that it holds no workflow under the ID makes a new setup; any other
/// answer or failure claims less: the question then names no asset.
fn retained(status: &Result<serde_json::Value>) -> bool {
    !matches!(status, Err(error) if error.config && error.message == "No retained owner workflow has this ID")
}
/// The question of a setup that the rail's journal holds under `id`.
fn resumed(network: &str, id: &str) -> String {
    format!(
        "Resume setup {id} of this agent's wallet account on {network}: a transaction that was signed before keeps its own asset, which is not shown here"
    )
}
/// The question of a new setup: its network and, with an amount and the asset of the new wallet,
/// what the setup moves into the wallet.
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
                "Create this agent's wallet account on {network} from the owner's account, fund it with {amount} {asset} plus the network's reserve and a fee float, and set its signers"
            )
        }
        _ => format!(
            "Create this agent's wallet account on {network} from the owner's account and set its signers"
        ),
    }
}
#[cfg(test)]
mod tests {
    use super::{question, resumed, retained};
    use allowit_sdk::error::Error;
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
            "Create this agent's wallet account on net:test from the owner's account, fund it with 20 COIN plus the network's reserve and a fee float, and set its signers"
        );
        assert_eq!(
            question(
                "net:test",
                Some("2.5"),
                Some(&asset("TOKEN", Some("ISSUER")))
            ),
            "Create this agent's wallet account on net:test from the owner's account, fund it with 2.5 TOKEN issued by ISSUER plus the network's reserve and a fee float, and set its signers"
        );
        assert_eq!(
            question("net:test", None, None),
            "Create this agent's wallet account on net:test from the owner's account and set its signers"
        );
    }
    /// Only the journal's answer that it holds no workflow under the ID makes a new setup.
    #[test]
    fn only_an_empty_journal_makes_a_new_setup() {
        assert!(!retained(&Err(Error::config(
            "No retained owner workflow has this ID"
        ))));
        assert!(retained(&Ok(serde_json::json!({"completed": false}))));
        assert!(retained(&Err(Error::config(
            "No native transaction is retained under this ID"
        ))));
        assert!(retained(&Err(Error::usage(
            "Invalid owner workflow request ID"
        ))));
    }
    /// A resume names its setup ID and network, and no asset or amount.
    #[test]
    fn a_resume_names_no_asset() {
        let text = resumed("net:test", "setup-0001");
        assert!(
            text.starts_with("Resume setup setup-0001 of this agent's wallet account on net:test")
        );
        assert!(!text.contains("fund it"));
    }
}
