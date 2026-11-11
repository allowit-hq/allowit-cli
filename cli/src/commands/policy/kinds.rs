//! `allowit policy kinds`: the wallet kinds of the selected rail, from the
//! SDK, or the kinds that the server accepts on that rail.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    kinds::Kind,
    routing, term,
};
use allowit_sdk::transport::{
    http::{HttpClient, Method, Request},
    rpc::Options,
};
use serde::Deserialize;
use std::time::Duration;

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit policy kinds
  allowit policy kinds --json
  allowit policy kinds --server";

/// List the wallet kinds of the selected rail: what enforces the policy and what holds the funds
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// List the kinds that the ALLOWIT_URL server accepts on the selected rail
    #[arg(long)]
    pub server: bool,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    let rail = cx.rail()?;
    let kinds: Vec<Kind> = if args.server {
        let server = cx.server_url()?;
        crate::flush_stderr(&mut cx.out.stderr);
        from_server(&server, rail.id)?
    } else {
        crate::kinds::supported(rail.id)?
            .into_iter()
            .map(Kind::from)
            .collect()
    };
    if cx.out.json {
        let value = serde_json::to_value(&kinds)
            .map_err(|_| Error::config("Cannot encode the wallet kinds"))?;
        cx.out.json_value(&value);
        return Ok(0);
    }
    // One block of rows per kind, in selection order, with one key width.
    let blocks: Vec<_> = kinds.iter().map(Kind::rows).collect();
    let width = blocks
        .iter()
        .flatten()
        .map(|(key, _)| key.len())
        .max()
        .unwrap_or(0);
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            cx.out.line("");
        }
        let padded: Vec<_> = block
            .iter()
            .map(|(key, text)| (format!("{key:width$}"), text.clone()))
            .collect();
        cx.out.stdout.push_str(&term::render(&padded));
    }
    Ok(0)
}

/// The server's answer to `GET /api/wallet-kinds?network=<caip2>`.
#[derive(Deserialize)]
struct Listing {
    network: String,
    kinds: Vec<Kind>,
}

/// A typed server refusal `{error, code}`.
#[derive(Deserialize)]
struct Refusal {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

/// Reads the kinds that the server accepts on `network`. The request carries
/// no credential. A kind id that this build does not know is an error, never
/// a fifth kind.
fn from_server(server: &str, network: &str) -> Result<Vec<Kind>> {
    let origin = crate::config::parse_origin(server)?;
    let url = format!("{origin}/api/wallet-kinds?network={network}");
    let client = HttpClient::new(Options {
        timeout: Duration::from_secs(20),
        max_response_bytes: 1 << 16,
        no_proxy: false,
        connect_timeout: Some(Duration::from_secs(10)),
    })
    .map_err(|_| Error::config("Cannot configure the server request"))?;
    allowit_sdk::progress::report("Reading the wallet kinds from the Allowit server");
    let response = client
        .send_refusal_body(
            Request {
                method: Method::Get,
                url: &url,
                headers: &[("accept".into(), "application/json".into())],
                body: None,
            },
            None,
        )
        .map_err(|_| unavailable(None))?;
    if !(200..300).contains(&response.status) {
        return Err(refusal(response.status, &response.body));
    }
    let listing: Listing = serde_json::from_slice(&response.body)
        .map_err(|_| Error::uncertain("The server returned an invalid wallet-kind list"))?;
    if routing::normalize(&listing.network).ok() != Some(network) {
        return Err(Error::uncertain(
            "The server listed the wallet kinds of another network",
        ));
    }
    let mut seen = vec![];
    for kind in &listing.kinds {
        if seen.contains(&kind.id) {
            return Err(Error::uncertain(
                "The server listed a wallet kind more than once",
            ));
        }
        seen.push(kind.id);
    }
    Ok(listing.kinds)
}

/// The error of a listing that got no answer, or a 503: exit 7. A listing
/// has no request ID, so the hint names only the retry.
fn unavailable(status: Option<u16>) -> Error {
    let mut error = Error::uncertain(
        "The Allowit server did not answer the wallet-kind request; check ALLOWIT_URL and retry",
    );
    error.code = allowit_sdk::transport_failure::NETWORK_UNAVAILABLE;
    error.status = status;
    error
}

/// The error of a refused request: the server's stable code and bounded text
/// when it sent a typed refusal, the HTTP status otherwise. A 503 without an
/// answer worth keeping is a network failure, which a retry may repair.
fn refusal(status: u16, body: &[u8]) -> Error {
    let reply: Option<Refusal> = serde_json::from_slice(body).ok();
    let bounded = |text: Option<String>, limit: usize| {
        text.filter(|t| !t.is_empty() && t.len() <= limit && !t.chars().any(char::is_control))
    };
    let code = bounded(reply.as_ref().and_then(|r| r.code.clone()), 64).filter(|code| {
        code.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    });
    let message = bounded(reply.and_then(|r| r.error), 300)
        .unwrap_or_else(|| "The wallet-kind request was refused".into());
    if status == 503 {
        let mut error = unavailable(Some(status));
        error.api_code = code;
        return error;
    }
    match code {
        Some(code) => Error::refused(status, code, message),
        None if status >= 500 => Error::uncertain(format!("Allowit returned HTTP {status}")),
        None => Error::api(status, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_keeps_the_server_code_and_maps_the_exit_code() {
        let typed = br#"{"error":"Unknown network","code":"network_unknown","details":{}}"#;
        let error = refusal(400, typed);
        assert_eq!(
            (error.code, error.api_code.as_deref()),
            (4, Some("network_unknown"))
        );
        assert!(error.message.ends_with("Unknown network"));
        // No typed body: 4xx is rejected, 5xx is uncertain, 503 is a network failure.
        assert_eq!(refusal(404, b"not found").code, 4);
        assert_eq!(refusal(401, b"").code, 3);
        assert_eq!(refusal(500, b"{}").code, 5);
        let down = refusal(503, br#"{"error":"down","code":"network_unavailable"}"#);
        assert_eq!(
            (down.code, down.api_code.as_deref()),
            (7, Some("network_unavailable"))
        );
        assert_eq!(refusal(503, b"").code, 7);
        // Server text reaches the terminal only as a bounded code and message.
        let hostile = br#"{"error":"line\u001b[31m","code":"Bad Code"}"#;
        let error = refusal(409, hostile);
        assert_eq!(error.api_code, None);
        assert!(
            error
                .message
                .ends_with("The wallet-kind request was refused")
        );
    }
}
