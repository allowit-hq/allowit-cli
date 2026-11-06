//! CLI file and terminal bindings for the shared public approval workflow.
use crate::{
    error::{Error, Result},
    routing,
};
use allowit_sdk_engine::Store;
pub use allowit_sdk_engine::vendors::stellar::runtime::approval::*;
use std::{io::Read, path::Path};
pub(crate) fn run(args: &[String], stdout: &mut String) -> Result<i32> {
    let usage = "allowit wallet approve --network stellar:testnet --policy-file PUBLIC_POLICY_JSON --owner G_ADDRESS --request-id ID";
    let mut flags = std::collections::BTreeMap::new();
    if !args.len().is_multiple_of(2) {
        return Err(Error::usage(usage));
    }
    for pair in args.as_chunks::<2>().0 {
        if !["--policy-file", "--owner", "--request-id"].contains(&pair[0].as_str())
            || flags.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err(Error::usage(usage));
        }
    }
    let get = |key| flags.get(key).copied().ok_or_else(|| Error::usage(usage));
    let network = routing::selected().ok_or_else(|| Error::usage(usage))?;
    check_network(&network)?;
    let file = std::fs::File::open(get("--policy-file")?)
        .map_err(|_| Error::config("Cannot read public Stellar policy file"))?;
    let mut bytes = Vec::new();
    file.take(131_073)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::config("Cannot read public Stellar policy file"))?;
    if bytes.len() > 131_072 {
        return Err(Error::config("Public Stellar policy file exceeds bounds"));
    }
    let policy: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| Error::config("Invalid public Stellar policy JSON"))?;
    let owner = get("--owner")?;
    let id = get("--request-id")?;
    approval_for_policy(&policy, owner, id)?;
    let store = Store::open(Path::new(".allowit/stellar-approval-journal"))?;
    let intent = prepare(&store, &policy, owner, id, &network)?;
    let url = signing_url(&crate::config::env_value("ALLOWIT_URL"), &intent)?;
    *stdout += &crate::output::json(
        &serde_json::json!({"state":"awaiting_owner_approval","executed":false,"movesFunds":false,"requestId":id,"signingUrl":url,"intentDigest":intent.intent_digest,"policyId":intent.policy_id}),
    );
    Ok(10)
}
