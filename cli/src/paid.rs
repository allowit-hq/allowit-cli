//! Paid calls: request a resource and, when the seller answers with a payment
//! challenge, pay it within the policy and present the proof. `call`, `curl`
//! and the `mcp` tools share this one path.
//!
//! The payment is an ordinary `exec` transfer: the same journal, the same
//! agent proof and the same policy. The server settles it; this file calls no
//! facilitator. A seller reply is data. Nothing in it is run or followed.
//!
//! No payment happens twice. A request ID names one challenge: the journal
//! keeps the challenge with the ID, and a retry with the same ID presents the
//! proof of the settled payment again. Every call prints its request ID on
//! stderr before it pays. The server also pays one seller challenge ID once
//! for a policy: a second request for it reads the original one.
use crate::{
    commands::exec,
    ctx::Ctx,
    error::{Error, Result},
    exit,
    rails::asset::{Asset, Use, units_decimal},
    request::request_id_valid,
    settings::journal,
};
use allowit_sdk::{
    execution::PaymentLimit,
    paid_service::{self, Challenge, SellerRequest, SellerResponse},
    runtime::Status,
};
use serde_json::{Value, json};
use std::time::Duration;

/// One request for a resource that may need payment.
pub(crate) struct Call {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// The most that the caller pays: `ASSET:AMOUNT`. A challenge without it
    /// is refused before any payment.
    pub max_price: Option<String>,
    pub request_id: Option<String>,
    pub wait: Duration,
}

/// The settled payment of a delivered resource.
pub(crate) struct Payment {
    pub request_id: String,
    pub network: &'static str,
    pub challenge: Challenge,
    /// The outcome document of the operation: transaction, settlement, the
    /// counted fee and the limits.
    pub receipt: Value,
    pub payment_limits: Vec<PaymentLimit>,
}

/// What the seller delivered, with the payment when one was needed.
pub(crate) struct Delivered {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub payment: Option<Payment>,
}

/// The price of a challenge as `exec --amount` text: `SYMBOL:DECIMAL`.
fn amount_text(challenge: &Challenge) -> Result<String> {
    let units = u128::try_from(challenge.units()?)
        .map_err(|_| paid_service::seller_failed("The seller's price is invalid."))?;
    Ok(format!(
        "{}:{}",
        challenge.amount.asset.symbol,
        units_decimal(units, challenge.amount.asset.decimals)
    ))
}

/// Refuse a price above the caller's bound, or a bound in another asset.
/// Nothing is paid.
fn within_bound(cx: &mut Ctx, challenge: &Challenge, max_price: Option<&str>) -> Result<()> {
    let price = &challenge.amount.text;
    let Some(max_price) = max_price else {
        return Err(Error::usage(format!(
            "the seller asks {price}; pass --max-price ASSET:AMOUNT to allow a payment"
        )));
    };
    let adapter = cx.adapter()?;
    let bound = adapter.parse_amount(max_price, Use::Spend)?;
    if bound.asset == Asset::Installed {
        return Err(Error::usage(
            "--max-price must name its asset: ASSET:AMOUNT",
        ));
    }
    // The same parser reads the seller's price, so both name the asset alike.
    let asked = adapter
        .parse_amount(&amount_text(challenge)?, Use::Spend)
        .map_err(|_| paid_service::seller_failed("The seller's price names an unknown asset."))?;
    if asked.asset != bound.asset || asked.decimals != bound.decimals {
        return Err(exit::price_exceeded(format!(
            "the seller asks {price}, and --max-price {max_price} names another asset; nothing was paid"
        )));
    }
    if asked.quantity > bound.quantity {
        return Err(exit::price_exceeded(format!(
            "the seller asks {price}, above --max-price {max_price}; nothing was paid"
        )));
    }
    Ok(())
}

/// The digest of the whole seller request: the method, the URL, the headers in
/// name order with the values of a repeated header in their order, and the
/// body. The payment of a request ID buys this request.
fn request_digest(request: &SellerRequest) -> String {
    let mut headers: Vec<(String, &str)> = request
        .headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.as_str()))
        .collect();
    // A stable sort by name: the values of one repeated header keep their order,
    // which can carry meaning.
    headers.sort_by(|a, b| a.0.cmp(&b.0));
    let mut bytes = Vec::new();
    let mut field = |part: &[u8]| {
        bytes.extend_from_slice(&(part.len() as u64).to_be_bytes());
        bytes.extend_from_slice(part);
    };
    field(request.method.as_bytes());
    field(request.url.as_bytes());
    for (name, value) in &headers {
        field(name.as_bytes());
        field(value.as_bytes());
    }
    field(b"body");
    field(request.body.as_deref().unwrap_or_default());
    allowit_sdk::digest(&bytes)
}

/// The server's code for a challenge that another request of the policy
/// already pays.
const CHALLENGE_PAID: &str = "challenge_already_paid";

/// The request ID that already pays `challenge`, after the server refused
/// `refused_id` for it: the ID in the refusal's details when the rail passes
/// them on, else the request of this journal that names the same challenge
/// and the same seller request and whose payment was `submitted`. A request
/// that the server refused or the policy denied is in the journal too, and it
/// paid nothing: only a submitted one is read. A payment made from another
/// journal, or for another seller request, is not presented here.
fn original_request(
    cx: &mut Ctx,
    error: &Error,
    challenge: &Challenge,
    request: &str,
    refused_id: &str,
    submitted: &mut dyn FnMut(&mut Ctx, &str) -> Result<bool>,
) -> Result<String> {
    if let Some(id) = error.detail("requestId").filter(|id| request_id_valid(id)) {
        return Ok(id.to_owned());
    }
    let dir = cx.dir()?;
    let mut original = None;
    for entry in journal::entries(&dir)? {
        let same = entry.request_id != refused_id
            && entry.paid_request.as_deref() == Some(request)
            && entry.challenge.as_ref().is_some_and(|paid| {
                paid.id.is_some()
                    && paid.id == challenge.id
                    && paid.pay_to == challenge.pay_to
                    && paid.network == challenge.network
            });
        if same && submitted(cx, &entry.request_id)? {
            original = Some(entry.request_id);
            break;
        }
    }
    original
        .ok_or_else(|| {
            let mut refused = Error::refused(
                409,
                CHALLENGE_PAID,
                "another request of this policy already pays this challenge, and this journal does not hold it; run the command again with the --request-id of that request",
            );
            refused.code = error.code;
            refused
        })
}

/// Pay `challenge` through `exec` under the policy, or resume its payment,
/// and return the confirmed outcome.
fn pay(
    cx: &mut Ctx,
    challenge: &Challenge,
    request: &str,
    request_id: String,
    wait: Duration,
) -> Result<(Value, Vec<PaymentLimit>, String)> {
    let planned = exec::plan(
        exec::RequestArgs {
            op: "transfer".into(),
            to: challenge.pay_to.clone(),
            amount: amount_text(challenge)?,
            token_out: None,
            args: None,
            request_id: Some(request_id.clone()),
            wait,
        },
        false,
        "call",
        Some(exec::Paid { challenge, request }),
        cx,
    )?;
    let adapter = planned.adapter;
    let result = adapter.exec_outcome(cx, &planned.request, planned.binding);
    let code = match &result {
        Ok((outcome, _)) => exit::execution_status(outcome.status.clone()).code(),
        Err(error) => error.code,
    };
    exec::finish(cx, &planned.dir, &request_id, code);
    let (outcome, payment, paid_by) = match result {
        Ok((outcome, payment)) => (outcome, payment, request_id),
        // The server already holds a request of this policy for this challenge.
        // Its original request ID pays it: read that one, and pay nothing.
        Err(error) if error.api_code.as_deref() == Some(CHALLENGE_PAID) => {
            let original =
                original_request(cx, &error, challenge, request, &request_id, &mut submitted)?;
            cx.out.progress(format!(
                "Request {original} already pays this challenge; reading it"
            ));
            let rail = cx.rail()?;
            let wallet = cx.wallet(rail)?;
            let binding = adapter.status_binding(cx, wallet.as_deref(), &original)?;
            let (outcome, payment) = adapter.status_outcome(cx, &original, binding, wait, None)?;
            (outcome, payment, original)
        }
        Err(error) => return Err(error),
    };
    if outcome.status != Status::Confirmed {
        let mut error = Error::uncertain(format!(
            "the payment of request {paid_by} is not confirmed; run the command again with --request-id {paid_by}: it resumes this payment and never pays twice"
        ));
        // A failed transfer paid nothing: the policy or the wallet refused it.
        error.code = exit::execution_status(outcome.status).code();
        return Err(error);
    }
    Ok((outcome.document, payment, paid_by))
}

fn delivered(reply: SellerResponse, payment: Option<Payment>) -> Delivered {
    Delivered {
        status: reply.status,
        content_type: reply.header("content-type").map(str::to_owned),
        body: reply.body,
        payment,
    }
}

/// Check the whole seller request before any payment, as the seller transport
/// checks it before a send. A request that could not be sent after the payment
/// must never be paid for.
fn check_request(request: &SellerRequest) -> Result<()> {
    paid_service::resource_url(&request.url)?;
    if request.url.contains('#') {
        return Err(Error::usage("The resource URL must have no fragment."));
    }
    if !matches!(
        request.method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
    ) {
        return Err(Error::usage(
            "The method must be GET, POST, PUT, PATCH or DELETE.",
        ));
    }
    let name = |name: &str| {
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    };
    let value = |value: &str| {
        value.len() <= 16_384 && value.bytes().all(|byte| (0x20..0x7f).contains(&byte))
    };
    if request.headers.len() > 32
        || request
            .headers
            .iter()
            .any(|(header, text)| !name(header) || !value(text))
    {
        return Err(Error::usage("A request header is invalid."));
    }
    if request
        .body
        .as_ref()
        .is_some_and(|body| body.len() > paid_service::MAX_BODY_BYTES)
    {
        return Err(Error::usage("The request body is too large."));
    }
    Ok(())
}

/// Whether the rail's journal holds a signed operation for `request_id`: its
/// payment was submitted, so only its status may be read.
fn submitted(cx: &mut Ctx, request_id: &str) -> Result<bool> {
    let rail = cx.rail()?;
    let wallet = cx.wallet(rail)?;
    let dir = cx.dir()?;
    Ok(
        allowit_sdk::execution::retained(&dir, rail.id, wallet.as_deref(), request_id)?
            .is_some_and(|operation| operation.signed()),
    )
}

/// The challenge that the journal keeps for `request_id`, when it names a
/// paid call for exactly this seller request: the same method, URL, headers
/// and body. Another request under the same ID is refused, so a payment is
/// never presented for work that it did not buy.
fn retained(cx: &mut Ctx, call: &Call, request: &str) -> Result<Option<Challenge>> {
    let Some(id) = &call.request_id else {
        return Ok(None);
    };
    if !request_id_valid(id) {
        return Err(Error::usage(
            "--request-id must be 8 to 100 characters from A-Z a-z 0-9 . _ : -",
        ));
    }
    let dir = cx.dir()?;
    let Some(entry) = journal::load(&dir, id)? else {
        return Ok(None);
    };
    match entry.challenge {
        Some(challenge)
            if challenge.resource == call.url && entry.paid_request.as_deref() == Some(request) =>
        {
            Ok(Some(challenge))
        }
        _ => Err(Error::usage(format!(
            "request ID {id} already names a different request; use a new --request-id"
        ))),
    }
}

/// Request the resource. On a payment challenge, check the price against the
/// caller's bound, pay within the policy and request the resource once more
/// with the proof.
pub(crate) fn fetch(cx: &mut Ctx, call: &Call) -> Result<Delivered> {
    let request = SellerRequest {
        method: call.method.clone(),
        url: call.url.clone(),
        headers: call.headers.clone(),
        body: call.body.clone(),
    };
    // Before any payment, on both paths: a request that cannot be sent is
    // never paid for.
    check_request(&request)?;
    let digest = request_digest(&request);
    // A request ID that the journal holds names its original challenge: the
    // seller is not asked for another one.
    let challenge = match retained(cx, call, &digest)? {
        Some(challenge) => {
            // A payment that was submitted is only read. One that was not would
            // start now, so the seller must still accept the challenge.
            let id = call.request_id.as_deref().unwrap_or_default();
            if !submitted(cx, id)? {
                paid_service::unexpired(&challenge, journal::now())?;
            }
            cx.out.progress(format!(
                "Resuming the payment of {} for this resource",
                challenge.amount.text
            ));
            challenge
        }
        None => {
            crate::flush_stderr(&mut cx.out.stderr);
            allowit_sdk::progress::report("Requesting the resource from the seller");
            let first = paid_service::send(&request, None)?;
            if first.status != 402 {
                return Ok(delivered(first, None));
            }
            let rail = cx.rail()?;
            paid_service::challenge(rail.id, &call.url, &first.reply())?
        }
    };
    // The price and its unit show before anything is paid.
    cx.out.progress(format!(
        "The seller asks {} to {} on {}",
        challenge.amount.text, challenge.pay_to, challenge.network
    ));
    within_bound(cx, &challenge, call.max_price.as_deref())?;
    // A new request ID prints before the payment, as `exec` prints it, so an
    // interrupted call can resume with `--request-id`.
    let request_id = match &call.request_id {
        Some(id) => id.clone(),
        None => {
            let id = journal::new_id()?;
            cx.out.generated_id(&id);
            id
        }
    };
    let rail = cx.rail()?;
    let (receipt, payment_limits, paid_by) = pay(cx, &challenge, &digest, request_id, call.wait)?;
    let proof = paid_service::proof(&challenge, &receipt)?;
    crate::flush_stderr(&mut cx.out.stderr);
    allowit_sdk::progress::report("Presenting the proof of payment to the seller");
    let second = paid_service::send(&request, Some(&proof)).map_err(|error| {
        paid_service::seller_failed(format!(
            "{} The payment settled as request {paid_by}; run the command again with --request-id {paid_by} to present its proof without paying again.",
            error.message
        ))
    })?;
    if !(200..300).contains(&second.status) {
        return Err(paid_service::seller_failed(format!(
            "the seller answered HTTP {} after the payment. The payment settled as request {paid_by}; run the command again with --request-id {paid_by} to present its proof without paying again.",
            second.status
        )));
    }
    Ok(delivered(
        second,
        Some(Payment {
            request_id: paid_by,
            network: rail.id,
            challenge,
            receipt,
            payment_limits,
        }),
    ))
}

/// The JSON document of a delivered resource. `body` is the text of the
/// resource, truncated to `max_body` bytes at a character boundary when
/// given; a resource that is not text has no `body`.
pub(crate) fn document(delivered: &Delivered, max_body: Option<usize>) -> Value {
    let mut value = json!({
        "status": delivered.status,
        "contentType": delivered.content_type,
        "bytes": delivered.body.len(),
    });
    match std::str::from_utf8(&delivered.body) {
        Ok(text) => {
            let mut end = max_body.map_or(text.len(), |max| max.min(text.len()));
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            value["body"] = text[..end].into();
            value["truncated"] = (end < text.len()).into();
        }
        Err(_) => {
            value["body"] = Value::Null;
            value["binary"] = true.into();
        }
    }
    if let Some(payment) = &delivered.payment {
        value["payment"] = json!({
            "requestId": payment.request_id,
            "network": payment.network,
            "price": payment.challenge.amount,
            "payTo": payment.challenge.pay_to,
            "resource": payment.challenge.resource,
            "challengeId": payment.challenge.id,
            "receipt": payment.receipt,
        });
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use allowit_sdk::{execution::Quantity, input::asset::AssetDisplay};

    fn challenge(id: Option<&str>) -> Challenge {
        Challenge {
            id: id.map(str::to_owned),
            resource: "https://seller.example/report".into(),
            network: "example:testnet".into(),
            scheme: "exact".into(),
            pay_to: "RECIPIENT".into(),
            amount: Quantity::new(
                2_500_000,
                AssetDisplay {
                    symbol: "XYZ".into(),
                    decimals: 7,
                    name: "Example".into(),
                    native: true,
                },
            ),
            asset: "XYZ".into(),
            expires_at: None,
            terms: Value::Null,
        }
    }

    #[test]
    fn the_price_of_a_challenge_is_exec_amount_text() {
        assert_eq!(amount_text(&challenge(None)).unwrap(), "XYZ:0.25");
        let mut zero = challenge(None);
        zero.amount.units = "0".into();
        assert!(amount_text(&zero).is_err());
    }

    #[test]
    fn a_price_above_the_bound_or_in_another_asset_is_refused_on_every_rail() {
        use allowit_sdk::plugins::{Feature, testkit};
        testkit::each(Feature::PaidService, |rail, fixtures| {
            let network = rail.test_support().network;
            let token = &fixtures.token;
            let mut cx = Ctx::new(
                crate::commands::Global {
                    rail: Some(network.into()),
                    config_dir: Some(crate::settings::temp_dir("paid-bound")),
                    ..Default::default()
                },
                crate::term::Out::new(false, true, false, true),
            );
            // The seller asks 0.25 of the rail's fixture token.
            let unit = 10u128.pow(u32::from(token.decimals));
            let mut asked = challenge(None);
            asked.network = network.into();
            asked.amount = Quantity::new(
                (unit / 4) as i128,
                AssetDisplay {
                    symbol: token.code.into(),
                    decimals: token.decimals,
                    name: token.code.into(),
                    native: token.native,
                },
            );
            let bound = |amount: &str| format!("{}:{amount}", token.code);
            // At or above the price: allowed. Nothing else is.
            within_bound(&mut cx, &asked, Some(&bound("0.25"))).unwrap();
            within_bound(&mut cx, &asked, Some(&bound("1"))).unwrap();
            let above = within_bound(&mut cx, &asked, Some(&bound("0.24"))).unwrap_err();
            assert_eq!(above.code, 9, "{}: {}", rail.id(), above.message);
            assert!(
                above.message.contains("nothing was paid"),
                "{}",
                above.message
            );
            // No bound, a bound without its asset and a bound that is no amount: usage errors.
            for bad in [None, Some("1"), Some("not-an-amount")] {
                let error = within_bound(&mut cx, &asked, bad).unwrap_err();
                assert_eq!(error.code, 2, "{}: {bad:?}", rail.id());
            }
            // A price in an asset that the rail does not know is the seller's failure.
            let mut unknown = asked.clone();
            unknown.amount.asset.symbol = "NOSUCHASSET".into();
            let error = within_bound(&mut cx, &unknown, Some(&bound("1"))).unwrap_err();
            assert_eq!(error.code, 8, "{}", rail.id());
        });
    }

    #[test]
    fn the_original_request_of_a_paid_challenge_comes_from_the_refusal_or_the_journal() {
        let dir = crate::settings::temp_dir("paid-original");
        let mut cx = Ctx::new(
            crate::commands::Global {
                config_dir: Some(dir),
                ..Default::default()
            },
            crate::term::Out::new(false, true, false, true),
        );
        let dir = cx.dir().unwrap();
        let entry =
            |id: &str, challenge: Option<Challenge>, request: Option<&str>| journal::Entry {
                request_id: id.into(),
                command: "call".into(),
                rail: "example:testnet".into(),
                wallet: None,
                policy: None,
                op: "transfer".into(),
                to: "RECIPIENT".into(),
                amount: "XYZ:0.25".into(),
                profile_version: None,
                server_origin: None,
                created_at: 1,
                exit: Some(0),
                challenge,
                paid_request: request.map(str::to_owned),
            };
        let paid = challenge(Some("c-0001"));
        for record in [
            entry("pay-original-1", Some(paid.clone()), Some("digest-a")),
            entry(
                "pay-other-0001",
                Some(challenge(Some("c-0002"))),
                Some("digest-a"),
            ),
            entry("pay-plain-0001", None, None),
        ] {
            journal::record(&dir, &record).unwrap();
        }
        // Later requests for the same challenge that the server refused: they are in the
        // journal, they paid nothing and they are newer than the request that settled.
        for (id, created_at) in [("pay-refused-01", 9), ("pay-refused-02", 8)] {
            let mut later = entry(id, Some(paid.clone()), Some("digest-a"));
            later.created_at = created_at;
            later.exit = Some(4);
            journal::record(&dir, &later).unwrap();
        }
        let refused = Error::refused(409, CHALLENGE_PAID, "already paid");
        // Only the original request has a submitted payment.
        let mut submitted = |_: &mut Ctx, id: &str| Ok(id == "pay-original-1");
        let mut find = |error: &Error, challenge: &Challenge, request: &str, refused_id: &str| {
            original_request(
                &mut cx,
                error,
                challenge,
                request,
                refused_id,
                &mut submitted,
            )
        };
        // The journal names the request that holds the same challenge and seller request.
        assert_eq!(
            find(&refused, &paid, "digest-a", "pay-second-01").unwrap(),
            "pay-original-1"
        );
        // A rail that passes the server's details on names the request directly.
        let named = refused
            .clone()
            .with_details(json!({"requestId": "pay-named-0001"}));
        assert_eq!(
            find(&named, &paid, "digest-a", "pay-second-01").unwrap(),
            "pay-named-0001"
        );
        // Another seller request, another challenge, another recipient or network, the
        // refused request itself and a challenge without an ID name no request: the
        // refusal stays an error with the server's code, and nothing is presented.
        let mut recipient = paid.clone();
        recipient.pay_to = "OTHER".into();
        let mut network = paid.clone();
        network.network = "example:mainnet".into();
        for (challenge, request, refused_id) in [
            (&paid, "digest-b", "pay-second-01"),
            (&challenge(Some("c-0009")), "digest-a", "pay-second-01"),
            (&recipient, "digest-a", "pay-second-01"),
            (&network, "digest-a", "pay-second-01"),
            (&paid, "digest-a", "pay-original-1"),
            (&challenge(None), "digest-a", "pay-second-01"),
        ] {
            let error = find(&refused, challenge, request, refused_id).unwrap_err();
            assert_eq!(error.api_code.as_deref(), Some(CHALLENGE_PAID));
            assert_eq!(error.code, refused.code);
        }
    }

    #[test]
    fn the_request_digest_covers_the_method_the_url_the_headers_and_the_body() {
        let base = SellerRequest {
            method: "POST".into(),
            url: "https://seller.example/translate".into(),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("Accept".into(), "text/plain".into()),
            ],
            body: Some(b"{\"text\":\"hello\"}".to_vec()),
        };
        let digest = request_digest(&base);
        assert_eq!(digest.len(), 64);
        // Header order and the case of a header name do not change the request.
        let mut reordered = base.clone();
        reordered.headers.reverse();
        reordered.headers[0].0 = "ACCEPT".into();
        assert_eq!(request_digest(&reordered), digest);
        // The values of one repeated header keep their order: another order is another
        // request.
        let mut repeated = base.clone();
        repeated
            .headers
            .push(("Accept".into(), "application/json".into()));
        let mut swapped = repeated.clone();
        swapped.headers.swap(1, 2);
        assert_ne!(request_digest(&repeated), request_digest(&swapped));
        // Every other difference is another request.
        let changed = |change: &dyn Fn(&mut SellerRequest)| {
            let mut request = base.clone();
            change(&mut request);
            assert_ne!(request_digest(&request), digest);
        };
        changed(&|r| r.method = "PUT".into());
        changed(&|r| r.url = "https://seller.example/translate?lang=fr".into());
        changed(&|r| r.body = Some(b"{\"text\":\"other\"}".to_vec()));
        changed(&|r| r.body = None);
        changed(&|r| r.headers[1].1 = "application/json".into());
        changed(&|r| r.headers.push(("X-Extra".into(), "1".into())));
        // Field boundaries are unambiguous.
        changed(&|r| {
            r.method = "POSThttps".into();
            r.url = "://seller.example/translate".into();
        });
    }

    #[test]
    fn a_request_that_cannot_be_sent_is_refused_before_any_payment() {
        let request = |change: &dyn Fn(&mut SellerRequest)| {
            let mut request = SellerRequest {
                method: "GET".into(),
                url: "https://seller.example/report".into(),
                headers: vec![("Accept".into(), "text/plain".into())],
                body: None,
            };
            change(&mut request);
            check_request(&request)
        };
        request(&|_| {}).unwrap();
        request(&|r| r.method = "DELETE".into()).unwrap();
        for refused in [
            request(&|r| r.method = "TRACE".into()),
            request(&|r| r.method = "get".into()),
            request(&|r| r.url = "https://seller.example/report#part".into()),
            request(&|r| r.url = "file:///etc/passwd".into()),
            request(&|r| r.headers = vec![("bad name".into(), "x".into())]),
            request(&|r| r.headers = vec![("X-A".into(), "a\r\nX-B: 1".into())]),
            request(&|r| r.headers = vec![("X-A".into(), "x".into()); 33]),
            request(&|r| r.body = Some(vec![0; paid_service::MAX_BODY_BYTES + 1])),
        ] {
            assert_eq!(refused.unwrap_err().code, 2);
        }
    }

    #[test]
    fn a_delivered_document_bounds_its_body_and_marks_what_it_cut() {
        let delivered = |body: &[u8]| Delivered {
            status: 200,
            content_type: Some("text/plain".into()),
            body: body.to_vec(),
            payment: None,
        };
        let whole = document(&delivered("résumé".as_bytes()), None);
        assert_eq!(whole["body"], "résumé");
        assert_eq!(whole["truncated"], false);
        assert_eq!(whole["bytes"], 8);
        // The cut falls on a character boundary, never inside a character.
        let cut = document(&delivered("résumé".as_bytes()), Some(2));
        assert_eq!(cut["body"], "r");
        assert_eq!(cut["truncated"], true);
        let binary = document(&delivered(&[0xff, 0xfe, 0x00]), Some(2));
        assert_eq!(binary["body"], Value::Null);
        assert_eq!(binary["binary"], true);
        assert!(whole.get("payment").is_none());
    }
}
