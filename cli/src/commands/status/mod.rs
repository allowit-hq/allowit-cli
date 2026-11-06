//! `allowit status ID`: reads one request without creating another.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    request::request_id_valid,
    routing,
    settings::journal,
    vendors,
};
use std::time::Duration;

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit status pay-0001
  allowit status pay-0001 --wait 60s
  allowit status pay-0001 --wallet WALLET --json";

/// Read a request's state. The local journal supplies its rail and wallet;
/// --rail and --wallet are the fallback
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// The request ID that exec printed
    pub id: String,
    /// Wait up to DUR for a final state
    #[arg(long, value_name = "DUR", default_value = "0", value_parser = super::parse_wait)]
    pub wait: Duration,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    if !request_id_valid(&args.id) {
        return Err(Error::usage(
            "ID must be the request ID that exec printed: 8 to 100 characters from A-Z a-z 0-9 . _ : -",
        ));
    }
    cx.out.request_id = Some(args.id.clone());
    let dir = cx.dir()?;
    let entry = journal::load(&dir, &args.id)?;
    let rail = match &entry {
        Some(e) => {
            let recorded = routing::info(&e.rail)?;
            // Only an explicit flag conflicts; ALLOWIT_RAIL and `rail use`
            // yield to the journal.
            if let Some(flag) = cx.global.rail.as_ref().filter(|_| cx.global.rail_flag)
                && routing::normalize(flag)? != recorded.id
            {
                return Err(Error::usage(format!(
                    "request {} belongs to {}",
                    args.id, recorded.id
                )));
            }
            recorded
        }
        None => cx.rail()?,
    };
    // The journal's wallet comes first; a different --wallet flag is refused,
    // as a different --rail flag is.
    let wallet = match &entry {
        Some(e) => {
            if let Some(flag) = cx.global.wallet.as_ref().filter(|_| cx.global.wallet_flag)
                && !e
                    .wallet
                    .as_ref()
                    .is_some_and(|w| vendors::address::same_wallet(rail.family, flag, w))
            {
                return Err(Error::usage(match &e.wallet {
                    Some(recorded) => format!("request {} belongs to wallet {recorded}", args.id),
                    None => format!("request {} names no wallet; drop --wallet", args.id),
                }));
            }
            // The rail resolved this wallet; its status check confirms it
            // against the installed profile.
            e.wallet.clone()
        }
        // A selection is checked as `--wallet` is.
        None => cx.wallet(rail)?,
    };
    let vendor = vendors::rail(rail);
    let code = routing::with_network(Some(rail.id.into()), || {
        let binding = vendor.status_binding(cx, wallet.as_deref(), &args.id)?;
        // A request reads only under the policy that sent it.
        if let Some(recorded) = entry.as_ref().and_then(|e| e.policy.as_deref())
            && binding.policy.as_deref() != Some(recorded)
        {
            return Err(Error::config(format!(
                "request {} was sent under policy {recorded}; the installed profile names {}",
                args.id,
                binding.policy.as_deref().unwrap_or("none")
            )));
        }
        vendor.status(
            cx,
            &args.id,
            binding,
            args.wait,
            entry.as_ref().and_then(|e| e.server_origin.as_deref()),
        )
    })?;
    if entry.is_some() {
        super::exec::finish(cx, &dir, &args.id, code);
    }
    Ok(code)
}
