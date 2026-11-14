//! `allowit call` and `allowit curl`: request a resource and pay the seller's
//! challenge within the policy. `call` also resolves a catalog entry.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    paid::{self, Call, Delivered},
};
use allowit_sdk::paid_service;
use serde_json::json;
use std::{path::PathBuf, time::Duration};

pub(crate) const CALL_EXAMPLES: &str = "\
Examples:
  allowit call ENTRY --max-price ASSET:0.5
  allowit call https://seller.example/report --max-price ASSET:0.5 --json
  allowit call ENTRY --max-price ASSET:0.5 --request-id call-0001 --wait 2m";
pub(crate) const CURL_EXAMPLES: &str = "\
Examples:
  allowit curl https://seller.example/report --max-price ASSET:0.5
  allowit curl https://seller.example/translate -X POST -d '{\"text\":\"hello\"}' -H 'content-type: application/json' --max-price ASSET:0.1
  allowit curl https://seller.example/report --max-price ASSET:0.5 -o report.json";

/// The request flags shared by `call` and `curl`.
#[derive(clap::Args, Debug)]
pub(crate) struct RequestArgs {
    /// The HTTP method: GET, POST, PUT, PATCH or DELETE [default: GET, or the entry's method]
    #[arg(short = 'X', long, value_name = "METHOD")]
    pub method: Option<String>,
    /// The request body
    #[arg(short = 'd', long, value_name = "DATA", conflicts_with = "data_file")]
    pub data: Option<String>,
    /// Read the request body from FILE, or from stdin for -
    #[arg(long, value_name = "FILE")]
    pub data_file: Option<PathBuf>,
    /// A request header as NAME: VALUE; repeat it for more
    #[arg(short = 'H', long = "header", value_name = "HEADER")]
    pub headers: Vec<String>,
    /// The most to pay, as ASSET:AMOUNT. Without it a seller that asks for payment is refused
    #[arg(long, value_name = "MAX")]
    pub max_price: Option<String>,
    /// Request ID of the payment; a retry reuses it and never pays twice [default: a new ID, printed on stderr]
    #[arg(long, value_name = "ID")]
    pub request_id: Option<String>,
    /// Bound status polling of the payment to DUR
    #[arg(long, value_name = "DUR", default_value = "60s", value_parser = super::parse_wait)]
    pub wait: Duration,
    /// Write the resource to FILE and print its summary
    #[arg(short = 'o', long, value_name = "FILE")]
    pub output: Option<PathBuf>,
}

/// Call a paid service from the catalog or by URL: pay what the seller asks, within the policy, and print the resource
#[derive(clap::Args, Debug)]
pub(crate) struct CallArgs {
    /// A catalog entry ID from skills, or the URL of the resource
    #[arg(value_name = "ENTRY|URL")]
    pub target: String,
    #[command(flatten)]
    pub request: RequestArgs,
}

/// Request a URL; on HTTP 402 pay within the policy and request it once more with the proof
#[derive(clap::Args, Debug)]
pub(crate) struct CurlArgs {
    /// The URL of the resource
    #[arg(value_name = "URL")]
    pub url: String,
    #[command(flatten)]
    pub request: RequestArgs,
}

fn is_url(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

/// Parse `NAME: VALUE` headers.
pub(crate) fn headers(raw: &[String]) -> Result<Vec<(String, String)>> {
    raw.iter()
        .map(|header| {
            header
                .split_once(':')
                .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
                .filter(|(name, _)| !name.is_empty())
                .ok_or_else(|| Error::usage("a header must be NAME: VALUE"))
        })
        .collect()
}

fn build(url: String, default_method: &str, args: &RequestArgs) -> Result<Call> {
    paid_service::resource_url(&url)?;
    let body = match (&args.data, &args.data_file) {
        (Some(data), _) => Some(data.clone().into_bytes()),
        (None, Some(path)) => Some(crate::ctx::read_input(path)?),
        (None, None) => None,
    };
    Ok(Call {
        method: args
            .method
            .as_deref()
            .unwrap_or(default_method)
            .to_ascii_uppercase(),
        url,
        headers: headers(&args.headers)?,
        body,
        max_price: args.max_price.clone(),
        request_id: args.request_id.clone(),
        wait: args.wait,
    })
}

/// Prints a delivered resource. JSON is one document with the status, the
/// body and the payment. Text prints the receipt of the payment on stderr
/// and the resource alone on stdout, so a pipe reads only the resource.
fn show(cx: &mut Ctx, delivered: &Delivered, output: Option<&PathBuf>) -> Result<i32> {
    if !(200..300).contains(&delivered.status) {
        return Err(paid_service::seller_failed(format!(
            "the seller answered HTTP {}",
            delivered.status
        )));
    }
    let mut document = paid::document(delivered, None);
    if let Some(path) = output {
        std::fs::write(path, &delivered.body)
            .map_err(|_| Error::config(format!("cannot write {}", path.display())))?;
        document
            .as_object_mut()
            .map(|document| document.remove("body"));
        document["output"] = json!(path.display().to_string());
    } else if std::str::from_utf8(&delivered.body).is_err() {
        return Err(Error::usage(
            "the resource is not text; write it to a file with --output FILE",
        ));
    }
    if cx.out.json {
        cx.out.json_value(&document);
        return Ok(0);
    }
    if let Some(payment) = &delivered.payment {
        // The receipt renders as exec prints it, on stderr.
        let mut receipt = crate::term::Out::new(false, true, false, true);
        crate::receipt::show(
            &mut receipt,
            payment.network,
            payment.receipt.clone(),
            &payment.payment_limits,
        );
        cx.out
            .progress(format!("paid  {}", payment.challenge.amount.text));
        cx.out.progress(receipt.stdout.trim_end());
    }
    match output {
        Some(path) => cx.out.progress(format!(
            "wrote  {} ({} bytes)",
            path.display(),
            delivered.body.len()
        )),
        None => {
            // The resource as the seller sent it. A pipe or a file gets its exact
            // bytes; a terminal gets display controls escaped, as all output does.
            cx.out.raw = !std::io::IsTerminal::is_terminal(&std::io::stdout());
            cx.out
                .stdout
                .push_str(&String::from_utf8_lossy(&delivered.body));
        }
    }
    Ok(0)
}

pub(crate) fn run_call(args: CallArgs, cx: &mut Ctx) -> Result<i32> {
    let (url, method) = if is_url(&args.target) {
        (args.target.clone(), "GET".to_owned())
    } else {
        let entry = super::skills::entry(cx, &args.target)?;
        cx.out
            .progress(format!("entry  {} ({})", entry.id, entry.name));
        let method = entry
            .methods
            .first()
            .cloned()
            .unwrap_or_else(|| "GET".into());
        (entry.resource, method)
    };
    let call = build(url, &method, &args.request)?;
    let delivered = paid::fetch(cx, &call)?;
    show(cx, &delivered, args.request.output.as_ref())
}

pub(crate) fn run_curl(args: CurlArgs, cx: &mut Ctx) -> Result<i32> {
    let call = build(args.url.clone(), "GET", &args.request)?;
    let delivered = paid::fetch(cx, &call)?;
    show(cx, &delivered, args.request.output.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_parse_and_a_header_without_a_name_is_refused() {
        assert_eq!(
            headers(&["Accept: text/plain".into(), "x-a:b:c".into()]).unwrap(),
            [
                ("Accept".to_owned(), "text/plain".to_owned()),
                ("x-a".to_owned(), "b:c".to_owned())
            ]
        );
        for bad in ["no-colon", ": value", ""] {
            assert_eq!(headers(&[bad.into()]).unwrap_err().code, 2, "{bad}");
        }
    }

    #[test]
    fn a_target_is_a_url_only_with_an_http_scheme() {
        assert!(is_url("https://seller.example/report"));
        assert!(is_url("http://127.0.0.1:18482/a"));
        for entry in [
            "sample-weather/forecast",
            "ftp://a",
            "file:///etc/passwd",
            "httpx",
        ] {
            assert!(!is_url(entry), "{entry}");
        }
    }
}
