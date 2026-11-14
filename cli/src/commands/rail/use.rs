//! `allowit rail use RAIL`.
use crate::{ctx::Ctx, error::Result, routing};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit rail use __ALLOWIT_EXAMPLE_RAIL__
  allowit rail use __ALLOWIT_EXAMPLE_RAIL__ --rpc-url URL";

/// Choose the current rail. Each rail keeps its own wallet, agent key and, with --rpc-url, its RPC endpoint
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// Rail as CAIP-2 or alias
    #[arg(value_name = "RAIL")]
    pub name: String,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let info = routing::info(&args.name)?;
    // `--rpc-url` on this command is kept as the rail's endpoint, so later
    // commands need no flag.
    let rpc_url = cx.global.rpc_url.clone();
    if let Some(url) = &rpc_url {
        let host = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"));
        if !host.is_some_and(|host| {
            !host.is_empty() && !host.chars().any(|c| c.is_whitespace() || c.is_control())
        }) {
            return Err(crate::error::Error::usage("--rpc-url takes an http(s) URL"));
        }
    }
    cx.settings()?.update(|config| {
        config.rail = Some(info.id.into());
        if let Some(url) = &rpc_url {
            config.rail_mut(info.id).rpc_url = Some(url.clone());
        }
        Ok(())
    })?;
    let mut value = serde_json::json!({"rail": info.id});
    if let Some(url) = rpc_url {
        value["rpcUrl"] = url.into();
    }
    cx.out.value(&value);
    Ok(0)
}
