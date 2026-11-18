//! Reads that the server answers only to a policy's agent. The agent key signs each one with
//! its rail's request proof, the bearer that `native/limits` takes: the proof binds the path,
//! the exact body and the time, so a policy id alone reads nothing.
use crate::error::{Error, Result};
use allowit_sdk::{
    transport::{
        http::{HttpClient, Method, Request},
        rpc::Options,
    },
    wallet::LocalSigner,
};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The route of the policy facts that only the policy's agent reads.
pub(crate) const POLICY_PATH: &str = "/api/agent/policy";
/// The route of the charge of one request of a policy.
pub(crate) const CHARGE_PATH: &str = "/api/agent/charge";
/// The server's code for a valid proof of a key that is not the policy's agent key.
pub(crate) const NOT_POLICY_AGENT: &str = "not_policy_agent";

/// How long one read may take.
pub(crate) struct Limits {
    pub timeout: Duration,
    pub connect: Duration,
    pub max_bytes: usize,
}

/// The status and body of a POST of `body` to `path` on `origin`, signed by `signer`. Only a
/// transport failure is an error: the caller reads every status.
pub(crate) fn post(
    origin: &str,
    path: &str,
    body: &Value,
    signer: &LocalSigner,
    limits: Limits,
) -> Result<(u16, Vec<u8>)> {
    let canonical = serde_json_canonicalizer::to_vec(body)
        .map_err(|_| Error::config("Cannot encode the request"))?;
    let bytes = serde_json::to_vec(body).map_err(|_| Error::config("Cannot encode the request"))?;
    let issued_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::config("The system clock is before 1970; set the clock"))?
        .as_secs();
    let protocol = allowit_sdk::plugins::for_network(signer.network())?.request_proof_protocol();
    let proof = allowit_sdk::portable::vendor::request_proof::authorize(
        protocol,
        signer.address(),
        issued_at,
        path,
        &canonical,
        |digest| signer.sign_hash(digest),
    )?;
    let client = HttpClient::new(Options {
        timeout: limits.timeout,
        max_response_bytes: limits.max_bytes,
        no_proxy: false,
        connect_timeout: Some(limits.connect),
    })
    .map_err(|_| Error::config("Cannot configure the server request"))?;
    let response = client
        .send_refusal_body(
            Request {
                method: Method::Post,
                url: &format!("{origin}{path}"),
                headers: &[
                    ("authorization".into(), proof),
                    ("content-type".into(), "application/json".into()),
                    ("accept".into(), "application/json".into()),
                ],
                body: Some(&bytes),
            },
            None,
        )
        .map_err(|_| Error::uncertain(format!("The Allowit server at {origin} did not answer")))?;
    Ok((response.status, response.body))
}

/// True when `status` and `body` are the server's refusal of a key that is not the policy's
/// agent key.
pub(crate) fn not_the_agent(status: u16, body: &[u8]) -> bool {
    status == 403
        && serde_json::from_slice::<Value>(body)
            .ok()
            .is_some_and(|body| body["code"] == NOT_POLICY_AGENT)
}
