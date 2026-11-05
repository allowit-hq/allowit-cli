//! `allowit keys generate NAME [--fund]`.
use crate::{ctx::Ctx, error::Result, settings::keys};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit keys generate agent
  allowit keys generate owner --fund";

/// Generate a private signing key for the selected rail
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Key name
    pub name: String,
    /// Fund the new account from the rail's test faucet
    #[arg(long)]
    pub fund: bool,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let vendor = cx.vendor()?;
    // Refuse a missing faucet before the key exists, so a retry is clean.
    if args.fund && !vendor.caps().verbs.contains(&"keys fund") {
        return Err(vendor.na("keys fund"));
    }
    let dir = cx.dir()?;
    let key = keys::generate(&dir, &args.name, rail.id)?;
    let view = super::view(&key);
    if !args.fund {
        cx.out.value(&view);
        return Ok(0);
    }
    if cx.out.json {
        // One JSON document: the key, then the faucet result.
        let mut funded = crate::term::Out::new(true, true, false, true);
        std::mem::swap(&mut cx.out, &mut funded);
        let result = vendor.keys_fund(cx, &key);
        std::mem::swap(&mut cx.out, &mut funded);
        let mut value = view;
        if let Ok(code) = &result {
            value["fund"] = serde_json::from_str(&funded.stdout).unwrap_or_default();
            cx.out.value(&value);
            return Ok(*code);
        }
        // The key exists even when the faucet fails: print it, then the error.
        cx.out.value(&value);
        return result;
    }
    cx.out.value(&view);
    cx.out.line("");
    vendor.keys_fund(cx, &key)
}
