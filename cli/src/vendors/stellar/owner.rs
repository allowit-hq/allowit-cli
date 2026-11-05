//! CLI bindings for the shared classic owner setup continuation.
use crate::{
    error::{Error, Result},
    routing,
};
use allowit_sdk_engine::Store;
pub use allowit_sdk_engine::vendors::stellar::runtime::owner::signing_url;
use allowit_sdk_engine::vendors::stellar::runtime::owner::{SetupRequest, prepare};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
fn clock() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|_| Error::config("System time is unavailable"))
}
pub(crate) fn run(args: &[String], stdout: &mut String) -> Result<i32> {
    let usage = "allowit wallet configure --network stellar:testnet --owner G_ADDRESS --agent G_ADDRESS --engine G_ADDRESS --request-id ID [--fee-per-operation 100]";
    let mut flags = BTreeMap::new();
    if !args.len().is_multiple_of(2) {
        return Err(Error::usage(usage));
    }
    for pair in args.as_chunks::<2>().0 {
        if ![
            "--owner",
            "--agent",
            "--engine",
            "--request-id",
            "--fee-per-operation",
        ]
        .contains(&pair[0].as_str())
            || flags.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err(Error::usage(usage));
        }
    }
    let required = |name| flags.get(name).copied().ok_or_else(|| Error::usage(usage));
    let request = SetupRequest {
        network: routing::selected().ok_or_else(|| Error::usage(usage))?,
        owner: required("--owner")?.into(),
        agent: required("--agent")?.into(),
        server: required("--engine")?.into(),
        request_id: required("--request-id")?.into(),
        fee_per_operation: flags
            .get("--fee-per-operation")
            .unwrap_or(&"100")
            .parse()
            .map_err(|_| Error::usage("Invalid setup fee"))?,
    };
    request.validate()?;
    let store = Store::open(Path::new(".allowit/stellar-owner-journal"))?;
    let setup = prepare(&store, &request, clock()?)?;
    let url = signing_url(&crate::config::env_value("ALLOWIT_URL"), &setup)?;
    let expired = setup.expires_at <= clock()?;
    *stdout += &crate::output::json(
        &serde_json::json!({"state":if expired{"owner_intent_expired"}else{"awaiting_owner_signature"},"executed":false,"recoveryOnly":expired,"requestId":request.request_id,"signingUrl":url,"intentDigest":setup.intent_digest,"transactionHash":setup.transaction_hash}),
    );
    Ok(10)
}
