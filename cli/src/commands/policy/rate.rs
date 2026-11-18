//! `allowit policy rate POLICY_ID REQUEST_ID`: the owner's reputation signal on one settled
//! request of their policy. Three signals stay apart: whether the service suited the task
//! (`suitability`), whether it delivered (`delivery`) and the quality of its output (`quality`).
//! The server reads the request and its subject from the wallet's journal. The same vote again
//! changes nothing; another vote replaces the earlier one and keeps its age. Without `--signal`
//! the command prints the owner's votes and the labels of the request's subject.
use crate::{
    commands::policy_help::is_policy_id,
    ctx::Ctx,
    error::{Error, Result},
    owner, term,
};
use allowit_sdk::marketplace::{self, Label, Labels, Populations, Signal, Vote};
use serde_json::Value;

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit policy rate POLICY_ID REQUEST_ID --signal delivery --vote bad --owner-key owner --origin http://localhost:5173
  allowit policy rate POLICY_ID REQUEST_ID --owner-key owner --origin http://localhost:5173 --json";

/// Rate a settled request of your policy: suitability, delivery or quality
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// The id of the policy
    pub policy_id: String,
    /// The request ID of the settled request
    pub request_id: String,
    /// What the vote is about
    #[arg(long, value_enum, requires = "vote")]
    pub signal: Option<SignalArg>,
    /// The vote on that signal
    #[arg(long, value_enum, requires = "signal")]
    pub vote: Option<VoteArg>,
    /// Named local owner key that signs the owner in
    #[arg(long)]
    pub owner_key: String,
    /// Owner sign-in Origin admitted by the backend
    #[arg(long, env = "ALLOWIT_OWNER_ORIGIN", hide_env_values = true)]
    pub origin: String,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub(crate) enum SignalArg {
    /// The service suited the task
    Suitability,
    /// The seller delivered what the payment bought
    Delivery,
    /// The output was good
    Quality,
}
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub(crate) enum VoteArg {
    Good,
    Bad,
}

fn label(label: &Label, who: &str) -> String {
    if label.is_unknown() {
        return format!("{who} unknown");
    }
    format!(
        "{who} {} (good {}, bad {})",
        label.support,
        label.good.unwrap_or(0),
        label.bad.unwrap_or(0)
    )
}
fn populations(populations: &Populations) -> String {
    let mut parts = Vec::new();
    if let Some(policy) = &populations.policy {
        parts.push(label(policy, "this policy"));
    }
    if let Some(owner) = &populations.owner {
        parts.push(label(owner, "your other policies"));
    }
    parts.push(label(&populations.others, "other owners"));
    parts.join(" · ")
}
fn text(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or("-")
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    if !is_policy_id(&args.policy_id) {
        return Err(Error::usage("POLICY_ID is not a policy id"));
    }
    let local = owner::load_for_server(cx, &args.owner_key)?;
    let context = local.context(&args.origin);
    crate::flush_stderr(&mut cx.out.stderr);
    let reply = match (args.signal, args.vote) {
        (Some(signal), Some(vote)) => {
            let signal = match signal {
                SignalArg::Suitability => Signal::Suitability,
                SignalArg::Delivery => Signal::Delivery,
                SignalArg::Quality => Signal::Quality,
            };
            let vote = match vote {
                VoteArg::Good => Vote::Good,
                VoteArg::Bad => Vote::Bad,
            };
            marketplace::contribute(&context, &args.policy_id, &args.request_id, signal, vote)?
        }
        _ => {
            let read = marketplace::votes(
                &context,
                &args.policy_id,
                std::slice::from_ref(&args.request_id),
            )?;
            read["requests"][&args.request_id].clone()
        }
    };
    if cx.out.json {
        cx.out.json_value(&reply);
        return Ok(0);
    }
    if reply.is_null() {
        return Err(Error::usage("This policy holds no request with this ID"));
    }
    let votes: std::collections::BTreeMap<Signal, Vote> =
        serde_json::from_value(reply["votes"].clone()).unwrap_or_default();
    let labels: Labels = serde_json::from_value(reply["labels"].clone()).unwrap_or_default();
    let vote_of = |signal: Signal| match votes.get(&signal) {
        Some(Vote::Good) => "good",
        Some(Vote::Bad) => "bad",
        None => "-",
    };
    let mut rows: Vec<(String, String)> = vec![
        ("request".into(), args.request_id.clone()),
        ("subject".into(), text(&reply["subject"])),
    ];
    if reply.get("outcome").is_some() {
        rows.push(("outcome".into(), text(&reply["outcome"])));
    }
    for (name, signal) in [
        ("suitability", Signal::Suitability),
        ("delivery", Signal::Delivery),
        ("quality", Signal::Quality),
    ] {
        rows.push((
            name.into(),
            format!(
                "your vote {} · {}",
                vote_of(signal),
                populations(labels.signal(signal))
            ),
        ));
    }
    cx.out.stdout.push_str(&term::render(&rows));
    Ok(0)
}
