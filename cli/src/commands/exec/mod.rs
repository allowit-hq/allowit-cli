//! `allowit exec`: one agent request, signed by the agent key and checked by
//! the wallet's policy on chain.
use crate::{
    ctx::Ctx,
    error::{Error, Result, quoted},
    request::request_id_valid,
    settings::journal,
    vendors::{ExecRequest, asset::exec_uses},
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
    /// transfer, swap, deposit, redeem, custom, custom.FN or custom:CONTRACT.FN
    #[arg(long, value_name = "OP")]
    pub op: String,
    /// The recipient address, or the vault of a deposit or redeem
    #[arg(long, value_name = "ADDR")]
    pub to: String,
    /// Amount in the selected rail's asset format, or INTEGER base units; INTEGER vault shares for redeem
    #[arg(long, value_name = "AMOUNT")]
    pub amount: String,
    /// The minimum to receive: ASSET:MIN for swap and redeem, INTEGER vault shares for deposit
    #[arg(long, value_name = "MIN")]
    pub token_out: Option<String>,
    /// Call arguments as a JSON object, for custom operations
    #[arg(long, value_name = "JSON")]
    pub args: Option<String>,
    /// Request ID; a retry reuses it [default: a new ID, printed on stderr]
    #[arg(long, value_name = "ID")]
    pub request_id: Option<String>,
    /// Wait up to DUR for a final result
    #[arg(long, value_name = "DUR", default_value = "60s", value_parser = super::parse_wait)]
    pub wait: Duration,
}

/// Submit an agent request: the policy checks it and the rail settles it
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    #[command(flatten)]
    pub request: RequestArgs,
    /// Print the request as JSON for a signer outside the CLI; the CLI never signs or submits it
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

/// Builds, journals and sends one `exec` request.
pub(crate) fn submit(a: RequestArgs, build_only: bool, cx: &mut Ctx) -> Result<i32> {
    check_op(&a.op)?;
    let rail = cx.rail()?;
    let vendor = cx.vendor()?;
    // Refuse what the rail cannot serve before a request ID is spent.
    if !vendor.caps().verbs.contains(&"exec") {
        return Err(vendor.na("exec"));
    }
    if build_only && !vendor.caps().exec_build_only {
        return Err(vendor.na("exec --build-only"));
    }
    crate::vendors::require_op(vendor.as_ref(), &a.op)?;
    // Input checks come before any settings, journal or network access.
    crate::vendors::address::check_recipient(rail.family, &a.to)?;
    let (spend, minimum) = exec_uses(&a.op);
    let amount = vendor.parse_amount(&a.amount, spend)?;
    let token_out = a
        .token_out
        .as_deref()
        .map(|t| vendor.parse_amount(t, minimum))
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
    vendor.check_exec(&a.op, &a.to, token_out.as_ref(), call_args.as_ref())?;
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
    let binding = vendor.exec_binding(cx, selected.as_deref(), &request_id)?;
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
                command: "exec".into(),
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
            },
        )?;
    }
    let result = vendor.exec(cx, &request, binding);
    if !build_only {
        let code = match &result {
            Ok(code) => *code,
            Err(e) => e.code,
        };
        finish(cx, &dir, &request_id, code);
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
            rail: "stellar:testnet".into(),
            wallet: None,
            policy: None,
            op: "transfer".into(),
            to: "G".into(),
            amount: "1".into(),
            profile_version: None,
            server_origin: None,
            created_at: 0,
            exit: None,
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
