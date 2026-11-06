//! Terminal bindings for shared owner workflows. Native validation belongs to the selected capability.
use crate::{
    ctx::{Ctx, read_input},
    error::{Error, Result},
    owner,
};
use allowit_sdk_engine::owner as shared;
use std::path::PathBuf;
pub(crate) const EXAMPLES: &str = "Examples:\n  allowit --rpc-url URL policy deploy draft.json --owner-key owner --origin http://localhost:5173 --agent-key agent --server ADDRESS --executable wallet.wasm --request-id deploy-0001 --fee-limit 2000000000 --yes";
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
    /// Reviewed executable for the selected wallet capability
    #[arg(long)]
    pub executable: PathBuf,
    /// Total fee ceiling in the selected rail's base units
    #[arg(long)]
    pub fee_limit: String,
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
pub(crate) fn deploy(a: Deploy, cx: &mut Ctx) -> Result<i32> {
    check(cx, &a.session.request_id)?;
    let local = owner::load(cx, &a.session.owner_key)?;
    let agent = cx.key(&a.agent_key)?;
    agent.check_rail(cx.rail()?.id)?;
    let draft = read_input(&a.policy_file)?;
    let executable = read_input(&a.executable)?;
    cx.confirm("Deploy this exact policy wallet using the owner account")?;
    let out = shared::deploy(
        &local.context(&a.session.origin),
        shared::Deploy {
            draft: &draft,
            agent: &agent.address,
            server: &a.server,
            executable: &executable,
            request_id: &a.session.request_id,
            fee_limit: &a.fee_limit,
        },
    )?;
    cx.out.value(&out);
    Ok(0)
}
pub(crate) fn fund(a: Fund, cx: &mut Ctx) -> Result<i32> {
    check(cx, &a.session.request_id)?;
    let local = owner::load(cx, &a.session.owner_key)?;
    let document = read_input(&a.policy_file)?;
    cx.confirm("Transfer this amount from the owner to the selected wallet")?;
    cx.out.value(&shared::fund(
        &local.context(&a.session.origin),
        &document,
        &a.session.request_id,
        &a.amount,
        &a.fee_limit,
    )?);
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
    let code = if out["transactions"]
        .as_array()
        .is_some_and(|v| !v.is_empty() && v.iter().all(|t| t["status"] == "confirmed"))
    {
        0
    } else {
        5
    };
    cx.out.value(&out);
    Ok(code)
}
