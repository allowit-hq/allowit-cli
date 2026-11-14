//! Terminal bindings for the owner's typed params: show and edit the values of a draft, change
//! the values of an active wallet with one owner signature, verify an installation on the
//! ledger and print the host and native placements of a policy. The shared SDK owns the checks.
use super::owner::Session;
use crate::{
    ctx::{Ctx, read_input},
    error::{Error, Result},
    exit::Exit,
    owner, term,
};
use allowit_sdk::owner as shared;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

pub(crate) const EXAMPLES: &str = "Examples:\n  allowit policy params show draft.json\n  allowit --json policy params set draft.json daily_spending_limit 5 --owner-key owner --origin http://localhost:5173";
pub(crate) const UPDATE_EXAMPLES: &str = "Examples:\n  allowit --rpc-url URL --json policy update wallet.json daily_spending_limit 2 --owner-key owner --origin http://localhost:5173 --request-id update-0001 --fee-limit 5000000 --yes";
pub(crate) const VERIFY_EXAMPLES: &str =
    "Examples:\n  allowit --rpc-url URL policy verify wallet.json --owner-key owner";

/// Show or change the typed values of an unsigned draft
#[derive(clap::Subcommand, Debug)]
pub(crate) enum Command {
    /// Print each param of a policy document in the unit of its asset
    Show(Show),
    /// Change typed values of an unsigned draft on the server; prints the next draft
    Set(Set),
}
#[derive(clap::Args, Debug)]
pub(crate) struct Show {
    /// Draft, or the outcome of a deployment, funding or update
    pub policy_file: PathBuf,
}
#[derive(clap::Args, Debug)]
pub(crate) struct Set {
    /// Generated public policy draft
    pub policy_file: PathBuf,
    /// Param key and its new value in human units; repeat the pair for more params
    #[arg(value_names = ["KEY", "VALUE"], num_args = 2.., required = true)]
    pub changes: Vec<String>,
    /// Named local owner key
    #[arg(long)]
    pub owner_key: String,
    /// Owner sign-in Origin admitted by the backend
    #[arg(long, env = "ALLOWIT_OWNER_ORIGIN", hide_env_values = true)]
    pub origin: String,
}
/// Revise an unsigned draft in the owner's words; prints the next draft
#[derive(clap::Args, Debug)]
pub(crate) struct Revise {
    /// Generated public policy draft
    pub policy_file: PathBuf,
    /// The change, in the owner's words
    #[arg(long)]
    pub instruction: String,
    /// Named local owner key
    #[arg(long)]
    pub owner_key: String,
    /// Owner sign-in Origin admitted by the backend
    #[arg(long, env = "ALLOWIT_OWNER_ORIGIN", hide_env_values = true)]
    pub origin: String,
}
/// Change values of an active wallet with one owner signature; usage and nonce stay
#[derive(clap::Args, Debug)]
pub(crate) struct Update {
    /// Outcome of the deployment, funding or last update of the wallet
    pub policy_file: PathBuf,
    /// Param key and its new value in human units; repeat the pair for more params
    #[arg(value_names = ["KEY", "VALUE"], num_args = 2.., required = true)]
    pub changes: Vec<String>,
    #[command(flatten)]
    pub session: Session,
    /// Total fee ceiling in the selected rail's base units
    #[arg(long)]
    pub fee_limit: String,
    /// Configuration revision that the owner reviewed, from policy verify
    #[arg(long, value_name = "N")]
    pub expected_revision: Option<u64>,
}
/// Compare a wallet document with the ledger and read the wallet state back
#[derive(clap::Args, Debug)]
pub(crate) struct Verify {
    /// Outcome of the deployment, funding or last update of the wallet
    pub policy_file: PathBuf,
    /// Named local owner key; verification signs nothing
    #[arg(long)]
    pub owner_key: String,
}
/// Print what the server evaluates and what the wallet enforces for a policy
#[derive(clap::Args, Debug)]
pub(crate) struct ExportPair {
    /// Draft, or the outcome of a deployment, funding or update
    pub policy_file: PathBuf,
}

pub(crate) fn run(command: Command, cx: &mut Ctx) -> Result<i32> {
    match command {
        Command::Show(a) => show(a, cx),
        Command::Set(a) => set(a, cx),
    }
}
/// The key and value pairs of the command line as one map. A key names one value.
fn pairs(words: &[String]) -> Result<BTreeMap<String, String>> {
    if words.is_empty() || !words.len().is_multiple_of(2) {
        return Err(Error::usage("Give each param key with its value"));
    }
    let mut edits = BTreeMap::new();
    for pair in words.chunks(2) {
        if edits.insert(pair[0].clone(), pair[1].clone()).is_some() {
            return Err(Error::usage(format!(
                "The param {} has two values",
                allowit_sdk::error::quoted(&pair[0])
            )));
        }
    }
    Ok(edits)
}
fn document(path: &std::path::Path) -> Result<(Vec<u8>, Value)> {
    let bytes = read_input(path)?;
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| Error::usage("The policy file must be a JSON document"))?;
    Ok((bytes, value))
}
fn now() -> Result<i64> {
    i64::try_from(owner::clock()?).map_err(|_| Error::config("The system clock is out of range"))
}
/// One row per change: the param and its value before and after, in asset units.
fn change_rows(changes: &[allowit_sdk::parameters::Change]) -> Vec<Value> {
    changes
        .iter()
        .map(|change| json!({"param": change.key, "before": change.before, "after": change.after}))
        .collect()
}
/// The params of a document as the owner reads them. A document that holds its policy under
/// `selection` shows the params of its installation.
fn param_lines(network: &str, document: &Value) -> Result<Vec<allowit_sdk::kinds::ParamLine>> {
    let selection = document.get("selection");
    let policy = selection.and_then(|selection| selection.get("policy"));
    let mut lines = allowit_sdk::kinds::param_lines(network, document);
    for candidate in [selection, policy].into_iter().flatten() {
        if lines.is_ok() {
            break;
        }
        lines = allowit_sdk::kinds::param_lines(network, candidate);
    }
    lines
}
fn show(a: Show, cx: &mut Ctx) -> Result<i32> {
    let network = cx.rail()?.id;
    let (_, document) = document(&a.policy_file)?;
    let lines = param_lines(network, &document)?;
    if cx.out.json {
        cx.out.json_value(&json!({"params": lines}));
    } else {
        let rows: Vec<_> = lines
            .iter()
            .map(|line| (format!("params.{}", line.key), line.value.clone()))
            .collect();
        cx.out.stdout.push_str(&term::render(&rows));
    }
    Ok(0)
}
/// Print the next draft as `policy generate` prints a draft: in JSON mode stdout is the draft
/// document, which `policy deploy` reads; text mode prints the changes first.
fn print_draft(cx: &mut Ctx, network: &str, changes: Vec<Value>, draft: &Value) {
    if cx.out.json {
        if cx.out.verbose && !cx.out.quiet {
            cx.out
                .stderr
                .push_str(&format!("{}\n", json!({"changes": changes})));
        }
        cx.out.json_value(draft);
        return;
    }
    let mut rows = vec![];
    term::flatten("changes".into(), &Value::Array(changes), &mut rows);
    if let Ok(lines) = param_lines(network, draft) {
        rows.extend(
            lines
                .into_iter()
                .map(|line| (format!("params.{}", line.key), line.value)),
        );
    }
    cx.out.stdout.push_str(&term::render(&rows));
    cx.out.line("");
    cx.out.text(draft);
}
fn set(a: Set, cx: &mut Ctx) -> Result<i32> {
    let network = cx.rail()?.id;
    let edits = pairs(&a.changes)?;
    let (draft, document) = document(&a.policy_file)?;
    // The values are parsed and checked before any key or network access.
    let (typed, changes) = shared::typed_changes(network, &document, &edits, now()?)?;
    let local = owner::load(cx, &a.owner_key)?;
    crate::flush_stderr(&mut cx.out.stderr);
    let next = shared::edit_params(
        &local.context(&a.origin),
        shared::EditParams {
            draft: &draft,
            changes: &typed,
        },
    )?;
    print_draft(cx, network, change_rows(&changes), &next);
    Ok(0)
}
pub(crate) fn revise(a: Revise, cx: &mut Ctx) -> Result<i32> {
    let network = cx.rail()?.id;
    let (draft, _) = document(&a.policy_file)?;
    let local = owner::load(cx, &a.owner_key)?;
    crate::flush_stderr(&mut cx.out.stderr);
    let next = shared::revise(
        &local.context(&a.origin),
        shared::Revise {
            draft: &draft,
            instruction: &a.instruction,
        },
    )?;
    print_draft(cx, network, vec![], &next);
    Ok(0)
}
pub(crate) fn update(a: Update, cx: &mut Ctx) -> Result<i32> {
    if !crate::request::request_id_valid(&a.session.request_id) {
        return Err(Error::usage("Invalid owner workflow ID"));
    }
    if cx.global.wallet_flag {
        return Err(Error::usage(
            "Owner commands use the explicit policy file. Omit --wallet.",
        ));
    }
    let network = cx.rail()?.id;
    let edits = pairs(&a.changes)?;
    let (bytes, document) = document(&a.policy_file)?;
    // The values are parsed and checked before any key or network access.
    let (typed, changes) = shared::typed_changes(network, &document, &edits, now()?)?;
    let local = owner::load(cx, &a.session.owner_key)?;
    cx.confirm("Change these values of the installed wallet with the owner account")?;
    let mut out = shared::update_params(
        &local.context(&a.session.origin),
        shared::UpdateParams {
            document: &bytes,
            changes: &typed,
            request_id: &a.session.request_id,
            expected_revision: a.expected_revision,
            fee_limit: &a.fee_limit,
        },
    )?;
    if let Some(fields) = out.as_object_mut() {
        fields.insert("changes".into(), Value::Array(change_rows(&changes)));
    }
    crate::receipt::show_owner(&mut cx.out, network, out);
    Ok(0)
}
pub(crate) fn verify(a: Verify, cx: &mut Ctx) -> Result<i32> {
    let (bytes, _) = document(&a.policy_file)?;
    let local = owner::load(cx, &a.owner_key)?;
    // Verification reads the owner's RPC only. It starts no owner session, so no Origin applies.
    let verification = shared::verify(&local.context(""), &bytes)?;
    let code = if verification.matches {
        Exit::Ok
    } else {
        Exit::Uncertain
    };
    let value = serde_json::to_value(&verification)
        .map_err(|_| Error::config("Cannot encode the verification"))?;
    if cx.out.json {
        cx.out.json_value(&value);
        return Ok(code.code());
    }
    let mut rows = vec![
        ("policyId".to_owned(), verification.policy_id.clone()),
        ("network".to_owned(), verification.network.clone()),
        ("matches".to_owned(), verification.matches.to_string()),
    ];
    for comparison in &verification.comparisons {
        rows.push((
            format!("compare.{}", comparison.field),
            if comparison.matches {
                "match"
            } else {
                "differs"
            }
            .to_owned(),
        ));
    }
    // A difference prints both sides, so the owner sees what the ledger holds.
    for comparison in verification.comparisons.iter().filter(|c| !c.matches) {
        term::flatten(
            format!("expected.{}", comparison.field),
            &comparison.expected,
            &mut rows,
        );
        term::flatten(
            format!("observed.{}", comparison.field),
            &comparison.observed,
            &mut rows,
        );
    }
    let mut state = value["state"].clone();
    // The installed params print in asset units when the rail reads them.
    if let Some(fields) = state.as_object_mut() {
        fields.remove("params");
    }
    term::flatten("state".into(), &state, &mut rows);
    for (key, value) in &verification.state.params {
        rows.push((
            format!("state.params.{key}"),
            term::text(&serde_json::to_value(value).unwrap_or(Value::Null))
                .trim_end()
                .replace('\n', ", "),
        ));
    }
    cx.out.stdout.push_str(&term::render(&rows));
    Ok(code.code())
}
pub(crate) fn export_pair(a: ExportPair, cx: &mut Ctx) -> Result<i32> {
    let network = cx.rail()?.id;
    let (bytes, _) = document(&a.policy_file)?;
    let pair = shared::paired_export(network, &bytes)?;
    let value =
        serde_json::to_value(&pair).map_err(|_| Error::config("Cannot encode the placements"))?;
    if cx.out.json {
        cx.out.json_value(&value);
        return Ok(0);
    }
    let list = |items: &[String]| {
        if items.is_empty() {
            "-".to_owned()
        } else {
            items.join(", ")
        }
    };
    let keys = |params: &BTreeMap<String, allowit_sdk::PolicyValue>| {
        list(&params.keys().cloned().collect::<Vec<_>>())
    };
    let rows = vec![
        ("policyId".to_owned(), pair.policy_id.clone()),
        ("network".to_owned(), pair.network.clone()),
        ("host.sourceHash".to_owned(), pair.host.source_hash.clone()),
        ("host.rules".to_owned(), list(&pair.host.rules)),
        (
            "host.evidenceFunctions".to_owned(),
            list(&pair.host.evidence_functions),
        ),
        ("host.params".to_owned(), keys(&pair.host.params)),
        (
            "native.walletKind".to_owned(),
            pair.native.wallet_kind.id().to_owned(),
        ),
        (
            "native.enforcement".to_owned(),
            value["native"]["enforcement"]
                .as_str()
                .unwrap_or("-")
                .to_owned(),
        ),
        ("native.rules".to_owned(), list(&pair.native.rules)),
        (
            "native.compilesSource".to_owned(),
            pair.native.source.is_some().to_string(),
        ),
        ("native.params".to_owned(), keys(&pair.native.params)),
    ];
    cx.out.stdout.push_str(&term::render(&rows));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_need_one_value_for_each_key() {
        let words = |list: &[&str]| list.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            pairs(&words(&["a", "1", "b", "2"])).unwrap(),
            BTreeMap::from([("a".into(), "1".into()), ("b".into(), "2".into())])
        );
        for refused in [&[][..], &["a"], &["a", "1", "b"], &["a", "1", "a", "2"]] {
            assert_eq!(pairs(&words(refused)).unwrap_err().code, 2);
        }
    }

    #[test]
    fn the_params_of_a_draft_and_of_an_outcome_print_in_asset_units() {
        allowit_sdk::plugins::testkit::each(
            allowit_sdk::plugins::Feature::DraftDisplay,
            |rail, fixtures| {
                let network = rail.test_support().network;
                let draft = fixtures
                    .owner_generation
                    .get("policy")
                    .unwrap_or(&fixtures.owner_generation)
                    .clone();
                let lines = param_lines(network, &draft).unwrap();
                assert!(!lines.is_empty());
                // An outcome holds its policy under `selection`.
                let outcome = json!({"requestId": "r", "selection": {"policy": draft}});
                assert_eq!(param_lines(network, &outcome).unwrap(), lines);
                assert!(param_lines(network, &json!({})).is_err());
            },
        );
    }

    #[test]
    fn a_change_row_names_the_param_and_both_values() {
        let rows = change_rows(&[allowit_sdk::parameters::Change {
            key: "daily_spending_limit".into(),
            noun: "daily spending".into(),
            before: "5 XYZ".into(),
            after: "2 XYZ".into(),
        }]);
        assert_eq!(
            rows,
            [json!({"param": "daily_spending_limit", "before": "5 XYZ", "after": "2 XYZ"})]
        );
    }
}
