//! `allowit exec`: one agent request, signed by the agent key and checked by
//! the wallet's policy on chain.
use crate::{
    ctx::Ctx,
    error::{Error, Result, quoted},
    rails::{ExecRequest, asset::exec_uses},
    request::request_id_valid,
    settings::journal,
};
use std::{path::Path, time::Duration};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit exec --op transfer --to DESTINATION --amount ASSET:5
  allowit exec --op transfer --to DESTINATION --amount ASSET:5 --request-id pay-0001 --wait 2m
  allowit exec --op transfer --to DESTINATION --amount 50000000 --build-only
  allowit exec --op deposit --to VAULT --amount ASSET:100 --token-out 990000000
  allowit exec --op redeem --to VAULT --amount 500000000 --token-out ASSET:49.5";

/// The request flags shared by `exec`.
#[derive(clap::Args, Debug)]
pub(crate) struct RequestArgs {
    /// An operation that `rail show` lists for the selected rail
    #[arg(long, value_name = "OP")]
    pub op: String,
    /// The recipient address, or the vault of a deposit or redeem
    #[arg(long, value_name = "ADDR")]
    pub to: String,
    /// Amount in the selected rail's asset format, or INTEGER base units; INTEGER vault shares for redeem
    #[arg(long, value_name = "AMOUNT")]
    pub amount: String,
    /// The minimum to receive: ASSET:MIN for redeem, INTEGER vault shares for deposit
    #[arg(long, value_name = "MIN")]
    pub token_out: Option<String>,
    /// Call arguments as a JSON object, for custom operations
    #[arg(long, value_name = "JSON")]
    pub args: Option<String>,
    /// Request ID; a retry reuses it [default: a new ID, printed on stderr]
    #[arg(long, value_name = "ID")]
    pub request_id: Option<String>,
    /// Bound status polling to DUR; prepare and submit are not bounded
    #[arg(long, value_name = "DUR", default_value = "60s", value_parser = super::parse_wait)]
    pub wait: Duration,
}

/// Submit an agent request: the policy checks it and the rail settles it
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    #[command(flatten)]
    pub request: RequestArgs,
    /// Print the unsigned request for a signer outside the CLI; the CLI never signs or submits it
    #[arg(long)]
    pub build_only: bool,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    submit(args.request, args.build_only, cx)
}

/// Checks the `--op` syntax. Which operations work depends on the rail.
pub(crate) fn check_op(op: &str) -> Result<()> {
    let ident = |s: &str| {
        !s.is_empty()
            && s.len() <= 64
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            && !s.starts_with(|c: char| c.is_ascii_digit())
    };
    let ok = matches!(op, "transfer" | "swap" | "deposit" | "redeem" | "custom")
        || op.strip_prefix("custom.").is_some_and(ident)
        || op.strip_prefix("custom:").is_some_and(|rest| {
            rest.rsplit_once('.')
                .is_some_and(|(contract, function)| !contract.is_empty() && ident(function))
        });
    if ok {
        Ok(())
    } else {
        Err(Error::usage(format!(
            "unknown operation {}; use transfer, swap, deposit, redeem, custom, custom.FN or custom:C.FN",
            quoted(op)
        )))
    }
}

/// One checked and journaled request, ready to send.
pub(crate) struct Planned {
    pub adapter: crate::rails::Rail,
    pub request: ExecRequest,
    pub binding: crate::rails::Binding,
    pub dir: std::path::PathBuf,
}

/// The seller's challenge that a request pays and the digest of the seller
/// request that the payment buys.
pub(crate) struct Paid<'a> {
    pub challenge: &'a allowit_sdk::paid_service::Challenge,
    pub request: &'a str,
}

/// Validates, builds and journals one request. Every input check runs here,
/// once; the rail adapter receives a checked request. `paid` is what the
/// request pays for, when it is a paid call: the journal keeps the challenge
/// and the seller request with the request ID, and the request carries the
/// context of the challenge.
pub(crate) fn plan(
    a: RequestArgs,
    build_only: bool,
    command: &str,
    paid: Option<Paid<'_>>,
    cx: &mut Ctx,
) -> Result<Planned> {
    let challenge = paid.as_ref().map(|paid| paid.challenge);
    check_op(&a.op)?;
    let rail = cx.rail()?;
    let adapter = cx.adapter()?;
    // Refuse what the rail cannot serve before a request ID is spent.
    if !adapter.caps().verbs.contains(&"exec") {
        return Err(adapter.na("exec"));
    }
    if build_only && !adapter.caps().exec_build_only {
        return Err(adapter.na("exec --build-only"));
    }
    crate::rails::require_op(&adapter, &a.op)?;
    // Input checks come before any settings, journal or network access.
    crate::rails::address::check_recipient(rail.family, &a.to)?;
    let (spend, minimum) = exec_uses(&a.op);
    let amount = adapter.parse_amount(&a.amount, spend)?;
    let token_out = a
        .token_out
        .as_deref()
        .map(|t| adapter.parse_amount(t, minimum))
        .transpose()?;
    let call_args = match &a.args {
        Some(raw) => {
            let value: serde_json::Value = serde_json::from_str(raw)
                .map_err(|_| Error::usage("--args must be a JSON object"))?;
            if !value.is_object() {
                return Err(Error::usage("--args must be a JSON object"));
            }
            Some(value)
        }
        None => None,
    };
    adapter.check_exec(&a.op, &a.to, token_out.as_ref(), call_args.as_ref())?;
    if a.request_id
        .as_deref()
        .is_some_and(|id| !request_id_valid(id))
    {
        return Err(Error::usage(
            "--request-id must be 8 to 100 characters from A-Z a-z 0-9 . _ : -",
        ));
    }
    let selected = cx.wallet(rail)?;
    let request_id = match a.request_id {
        Some(id) => id,
        None => {
            let id = journal::new_id()?;
            cx.out.generated_id(&id);
            id
        }
    };
    let binding = adapter.exec_binding(cx, selected.as_deref(), &request_id)?;
    cx.out.request_id = Some(request_id.clone());
    let request = ExecRequest {
        op: a.op,
        to: a.to,
        amount,
        token_out,
        args: call_args,
        request_id: request_id.clone(),
        wait: a.wait,
        build_only,
        wallet: binding.wallet.clone(),
        paid_service: challenge.map(allowit_sdk::paid_service::Challenge::context),
    };
    cx.out.detail(format!(
        "wallet  {}",
        request.wallet.as_deref().unwrap_or("-")
    ));
    let dir = cx.dir()?;
    if !build_only {
        journal::record(
            &dir,
            &journal::Entry {
                request_id: request_id.clone(),
                command: command.into(),
                rail: rail.id.into(),
                wallet: binding.wallet.clone(),
                policy: binding.policy.clone(),
                op: request.op.clone(),
                to: request.to.clone(),
                amount: request.amount.text.clone(),
                created_at: journal::now(),
                exit: None,
                profile_version: binding.profile.as_ref().map(|p| p.version.clone()),
                server_origin: binding.profile.as_ref().map(|p| p.server_origin().into()),
                challenge: challenge.cloned(),
                paid_request: paid.as_ref().map(|paid| paid.request.to_owned()),
            },
        )?;
    }
    Ok(Planned {
        adapter,
        request,
        binding,
        dir,
    })
}

/// Validates, builds, journals and sends one `exec` request, and prints its
/// receipt.
pub(crate) fn submit(a: RequestArgs, build_only: bool, cx: &mut Ctx) -> Result<i32> {
    let planned = plan(a, build_only, "exec", None, cx)?;
    let result = planned.adapter.exec(cx, &planned.request, planned.binding);
    if !build_only {
        let code = match &result {
            Ok(code) => *code,
            Err(e) => e.code,
        };
        finish(cx, &planned.dir, &planned.request.request_id, code);
    }
    result
}

/// Records a request's last exit code. A journal failure never replaces the
/// command's result; it is reported as a warning.
pub(crate) fn finish(cx: &mut Ctx, dir: &Path, id: &str, code: i32) {
    if let Err(e) = journal::finish(dir, id, code) {
        cx.out.warning(format!(
            "the journal did not record the result of {id}: {}",
            e.message
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commands::Global, term::Out};

    #[test]
    fn a_locked_journal_keeps_the_result_and_warns() {
        let dir = crate::settings::temp_dir("finish");
        let mut cx = Ctx::new(Global::default(), Out::new(false, true, false, true));
        let entry = journal::Entry {
            request_id: "pay-0001".into(),
            command: "exec".into(),
            rail: "example:testnet".into(),
            wallet: None,
            policy: None,
            op: "transfer".into(),
            to: "G".into(),
            amount: "1".into(),
            profile_version: None,
            server_origin: None,
            created_at: 0,
            exit: None,
            challenge: None,
            paid_request: None,
        };
        journal::record(&dir, &entry).unwrap();
        let held = journal::lock(&dir, "pay-0001").unwrap();
        finish(&mut cx, &dir, "pay-0001", 4);
        assert_eq!(
            cx.out.stderr,
            "warning: the journal did not record the result of pay-0001: the request journal is unsafe or locked\n"
        );
        drop(held);
        finish(&mut cx, &dir, "pay-0001", 4);
        assert_eq!(
            journal::load(&dir, "pay-0001").unwrap().unwrap().exit,
            Some(4)
        );
    }

    #[test]
    fn op_syntax() {
        for ok in [
            "transfer",
            "swap",
            "custom",
            "custom.deposit_all",
            "custom:CABC.run",
        ] {
            super::check_op(ok).unwrap();
        }
        for bad in [
            "",
            "Transfer",
            "custom.",
            "custom:.f",
            "custom:C",
            "custom.1x",
            "pay",
        ] {
            assert_eq!(super::check_op(bad).unwrap_err().code, 2, "{bad}");
        }
    }
}
