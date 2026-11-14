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

/// Pay `challenge` through `exec` under the policy, or resume its payment,
/// and return the confirmed outcome.
fn pay(
    cx: &mut Ctx,
    challenge: &Challenge,
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
        Some(challenge),
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
        Err(error)
            if error.api_code.as_deref() == Some("challenge_already_paid")
                && error.detail("requestId").is_some_and(request_id_valid) =>
        {
            let original = error.detail("requestId").unwrap_or_default().to_owned();
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
/// paid call for this resource.
fn retained(cx: &mut Ctx, call: &Call) -> Result<Option<Challenge>> {
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
        Some(challenge) if challenge.resource == call.url => Ok(Some(challenge)),
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
    // A request ID that the journal holds names its original challenge: the
    // seller is not asked for another one.
    let challenge = match retained(cx, call)? {
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
    let (receipt, payment_limits, paid_by) = pay(cx, &challenge, request_id, call.wait)?;
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
