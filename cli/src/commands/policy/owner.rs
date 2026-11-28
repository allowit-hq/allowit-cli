//! Terminal bindings for shared owner workflows. Native validation belongs to the selected capability.
use crate::{
    ctx::{Ctx, read_input},
    error::{Error, Result},
    owner,
};
use allowit_sdk::owner as shared;
use std::path::PathBuf;
pub(crate) const EXAMPLES: &str = "Examples:\n  allowit --rpc-url URL policy deploy draft.json --owner-key owner --origin http://localhost:5173 --agent-key agent --server ADDRESS --executable wallet.code --amount 10 --request-id deploy-0001 --fee-limit 2000000000 --yes\n  allowit --rpc-url URL policy deploy native-draft.json --owner-key owner --origin http://localhost:5173 --agent-key agent --server ADDRESS --amount 10 --request-id deploy-0002 --fee-limit 2000000000 --yes";
#[derive(clap::Args, Debug)]
pub(crate) struct Session {
    /// Named local owner key, independent of the selected agent key
    #[arg(long)]
    pub owner_key: String,
    /// Owner sign-in Origin admitted by the backend
    #[arg(long, env = "ALLOWIT_OWNER_ORIGIN", hide_env_values = true)]
    pub origin: String,
    /// Stable workflow ID; resume with the same ID and arguments
    #[arg(long)]
    pub request_id: String,
}
#[derive(clap::Args, Debug)]
pub(crate) struct Deploy {
    /// Generated public policy draft
    pub policy_file: PathBuf,
    #[command(flatten)]
    pub session: Session,
    /// Explicit named agent key; agent selection does not change
    #[arg(long)]
    pub agent_key: String,
    /// Independent server signing address
    #[arg(long)]
    pub server: String,
    /// Reviewed executable of a contract wallet; a Native wallet is an account and takes none
    #[arg(long)]
    pub executable: Option<PathBuf>,
    /// Total fee ceiling in the selected rail's base units
    #[arg(long)]
    pub fee_limit: String,
    /// Positive decimal amount of the installed asset that the deployment moves into the wallet
    #[arg(long)]
    pub amount: Option<String>,
    /// Positive decimal amount of the network's fee asset that the deployment moves into a wallet
    /// of another asset, from which the wallet repays network fees
    #[arg(long)]
    pub fee_asset_amount: Option<String>,
}
#[derive(clap::Args, Debug)]
pub(crate) struct Fund {
    /// Public selection from deployment
    pub policy_file: PathBuf,
    #[command(flatten)]
    pub session: Session,
    /// Positive decimal amount of the installed asset
    #[arg(long)]
    pub amount: String,
    /// Total fee ceiling in the selected rail's base units
    #[arg(long)]
    pub fee_limit: String,
}
#[derive(clap::Args, Debug)]
pub(crate) struct Activate {
    /// Public deployment selection or generated account policy draft
    pub policy_file: PathBuf,
    #[command(flatten)]
    pub session: Session,
    /// Named agent key when the draft has no deployment selection
    #[arg(long, requires = "server")]
    pub agent_key: Option<String>,
    /// Independent server address when the draft has no deployment selection
    #[arg(long, requires = "agent_key")]
    pub server: Option<String>,
}
#[derive(clap::Args, Debug)]
pub(crate) struct Export {
    /// Public selection from activation
    pub policy_file: PathBuf,
    /// Named local owner key
    #[arg(long)]
    pub owner_key: String,
    /// Owner sign-in Origin admitted by the backend
    #[arg(long, env = "ALLOWIT_OWNER_ORIGIN", hide_env_values = true)]
    pub origin: String,
    /// Completed local activation ID
    #[arg(long)]
    pub activation_id: String,
}
fn check(cx: &mut Ctx, id: &str) -> Result<()> {
    if !crate::request::request_id_valid(id) {
        return Err(Error::usage("Invalid owner workflow ID"));
    }
    if cx.global.wallet_flag {
        return Err(Error::usage(
            "Owner commands use the explicit policy file. Omit --wallet.",
        ));
    }
    Ok(())
}
/// The question of a deployment. A Native deployment creates the agent's wallet account with one
/// owner setup, which moves the amount on top of the network's reserve and a fee float: the owner
/// confirms the network and all of it, as `wallet setup` asks.
fn deploy_question(native: bool, network: &str, amount: Option<&str>) -> String {
    if !native {
        return "Deploy and fund this exact policy wallet using the owner account".into();
    }
    let funding = match amount {
        Some(amount) => {
            format!("{amount} of the wallet's asset plus the network's reserve and a fee float")
        }
        None => "the network's reserve and a fee float".into(),
    };
    format!(
        "Create this agent's wallet account on {network} from the owner's account, fund it with {funding}, set its signers, and bind this exact policy to it"
    )
}
pub(crate) fn deploy(a: Deploy, cx: &mut Ctx) -> Result<i32> {
    check(cx, &a.session.request_id)?;
    let local = owner::load(cx, &a.session.owner_key)?;
    let agent = cx.key(&a.agent_key)?;
    agent.check_rail(cx.rail()?.id)?;
    let draft = read_input(&a.policy_file)?;
    // A Native wallet is the agent's own account: its deployment creates and funds the account
    // and binds the draft to it, and the rail refuses an executable for it.
    let native = serde_json::from_slice::<serde_json::Value>(&draft)
        .ok()
        .and_then(|draft| allowit_sdk::generation::wallet_kind_of(&draft))
        == Some(allowit_sdk::generation::WalletKind::Native);
    // The confirmation follows the draft's kind as this command reads it; the rail signs by the
    // kind of the draft's runtime. Each case where the two could differ is refused before a key,
    // the network or the confirmation, so the owner never confirms one action and signs another:
    // - a Native draft takes no executable: a draft whose runtime is a contract wallet then
    //   reaches the rail without code, which the rail refuses before anything is signed;
    // - a contract wallet draft takes a non-empty executable: a draft whose runtime is a Native
    //   wallet then reaches the rail with code, which the rail refuses before anything is signed.
    let executable = match (&a.executable, native) {
        (Some(_), true) => {
            return Err(Error::usage(
                "A Native wallet is an account and runs no wallet code: deploy it without --executable",
            ));
        }
        (Some(path), false) => match read_input(path)? {
            code if code.is_empty() => {
                return Err(Error::usage(format!(
                    "{} is empty: name the reviewed wallet code with --executable",
                    path.display()
                )));
            }
            code => code,
        },
        (None, true) => Vec::new(),
        (None, false) => {
            return Err(Error::usage(
                "A contract wallet deploys reviewed code: name it with --executable",
            ));
        }
    };
    let context = local.context(&a.session.origin);
    cx.confirm(&deploy_question(
        native,
        context.network,
        a.amount.as_deref(),
    ))?;
    let network = cx.rail()?.id;
    let out = shared::deploy(
        &context,
        shared::Deploy {
            draft: &draft,
            agent: &agent.address,
            server: &a.server,
            executable: &executable,
            request_id: &a.session.request_id,
            fee_limit: &a.fee_limit,
            amount: a.amount.as_deref(),
            fee_asset_amount: a.fee_asset_amount.as_deref(),
        },
    )?;
    crate::receipt::show_owner(&mut cx.out, network, out);
    Ok(0)
}
pub(crate) fn fund(a: Fund, cx: &mut Ctx) -> Result<i32> {
    check(cx, &a.session.request_id)?;
    let local = owner::load(cx, &a.session.owner_key)?;
    let document = read_input(&a.policy_file)?;
    cx.confirm("Transfer this amount from the owner to the selected wallet")?;
    let network = cx.rail()?.id;
    let out = shared::fund(
        &local.context(&a.session.origin),
        &document,
        &a.session.request_id,
        &a.amount,
        &a.fee_limit,
    )?;
    crate::receipt::show_owner(&mut cx.out, network, out);
    Ok(0)
}
pub(crate) fn activate(a: Activate, cx: &mut Ctx) -> Result<i32> {
    check(cx, &a.session.request_id)?;
    let local = owner::load(cx, &a.session.owner_key)?;
    let document = read_input(&a.policy_file)?;
    let agent = if let Some(name) = a.agent_key {
        let key = cx.key(&name)?;
        key.check_rail(cx.rail()?.id)?;
        Some(key.address)
    } else {
        None
    };
    cx.confirm("Activate this exact policy source and parameters for the selected agent")?;
    let out = shared::activate(
        &local.context(&a.session.origin),
        shared::Activate {
            document: &document,
            agent: agent.as_deref(),
            server: a.server.as_deref(),
            request_id: &a.session.request_id,
        },
    )?;
    cx.out.value(&out);
    Ok(0)
}
pub(crate) fn export(a: Export, cx: &mut Ctx) -> Result<i32> {
    let local = owner::load(cx, &a.owner_key)?;
    let document = read_input(&a.policy_file)?;
    let profile = shared::export(&local.context(&a.origin), &document, &a.activation_id)?;
    cx.out.value(&profile.document()?);
    Ok(0)
}
/// Read original owner receipts without signing or submitting
#[derive(clap::Args, Debug)]
pub(crate) struct Status {
    pub request_id: String,
}
pub(crate) fn status(a: Status, cx: &mut Ctx) -> Result<i32> {
    let out = owner::status(cx, &a.request_id)?;
    let code = status_exit(&out).code();
    let network = cx.rail()?.id;
    crate::receipt::show_owner(&mut cx.out, network, out);
    Ok(code)
}
/// The exit of an owner status: 0 when every transaction is confirmed; 4, as the owner step
/// itself exits, when one failed on the ledger (the step is over: a new request ID starts again);
/// otherwise 5, ask again with the same request ID.
fn status_exit(document: &serde_json::Value) -> crate::exit::Exit {
    use crate::exit::Exit;
    let statuses: Vec<_> = document["transactions"]
        .as_array()
        .map(|items| items.iter().map(|t| &t["status"]).collect())
        .unwrap_or_default();
    if statuses.iter().any(|status| *status == "failed") {
        Exit::Rejected
    } else if !statuses.is_empty() && statuses.iter().all(|status| *status == "confirmed") {
        Exit::Ok
    } else {
        Exit::Uncertain
    }
}
#[cfg(test)]
mod tests {
    use super::{deploy_question, status_exit};
    use crate::exit::Exit;
    use serde_json::json;

    /// A confirmed step exits 0, a failed one 4 like the step itself, anything else 5.
    #[test]
    fn an_owner_status_exits_by_its_transactions() {
        let of = |statuses: &[&str]| {
            status_exit(&json!({"transactions": statuses
                .iter()
                .map(|s| json!({"status": s}))
                .collect::<Vec<_>>()}))
        };
        assert_eq!(of(&["confirmed"]), Exit::Ok);
        assert_eq!(of(&["confirmed", "confirmed"]), Exit::Ok);
        assert_eq!(of(&["failed"]), Exit::Rejected);
        assert_eq!(of(&["confirmed", "failed"]), Exit::Rejected);
        assert_eq!(of(&["submitted"]), Exit::Uncertain);
        assert_eq!(of(&["submissionUnknown"]), Exit::Uncertain);
        assert_eq!(of(&[]), Exit::Uncertain);
        assert_eq!(status_exit(&json!({})), Exit::Uncertain);
    }

    /// The owner of a Native deployment confirms the network, the amount and the reserve with the
    /// fee float that its setup moves; a contract wallet's question does not change.
    #[test]
    fn a_native_deployment_names_what_its_setup_moves() {
        assert_eq!(
            deploy_question(true, "net:test", Some("20")),
            "Create this agent's wallet account on net:test from the owner's account, fund it with 20 of the wallet's asset plus the network's reserve and a fee float, set its signers, and bind this exact policy to it"
        );
        assert_eq!(
            deploy_question(true, "net:test", None),
            "Create this agent's wallet account on net:test from the owner's account, fund it with the network's reserve and a fee float, set its signers, and bind this exact policy to it"
        );
        assert_eq!(
            deploy_question(false, "net:test", Some("20")),
            "Deploy and fund this exact policy wallet using the owner account"
        );
    }
}
