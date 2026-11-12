//! The receipt of an agent request: what settled, what it cost and what
//! remains of each spending limit. Every amount is a quantity that carries
//! its own unit text, from the SDK or the server; this file never names a unit.
//! It also prints the transactions of an owner workflow.
use crate::term::{self, Out};
use allowit_sdk::{
    execution::{
        EXPLORER_KEY as EXPLORER, FEE_CHARGED_KEY as FEE_CHARGED, LimitReport, Limits,
        PaymentLimit, Settlement, WalletLimits, explorer, fee_charged,
    },
    runtime::owner::{LinkKind, explorer_link},
};
use serde_json::Value;

const TRANSACTION: &str = "transactionHash";
/// The key of the verified receipts in an owner workflow document.
const RECEIPTS: &str = "receipts";
/// The key of the transactions in an owner workflow document: hashes, or
/// one object for each transaction.
const TRANSACTIONS: &str = "transactions";
/// The per-payment limits to print beside `reported`. A limit that the rail
/// reports with counters is not repeated.
fn uncounted<'a>(payment: &'a [PaymentLimit], reported: &[LimitReport]) -> Vec<&'a PaymentLimit> {
    payment
        .iter()
        .filter(|limit| !reported.iter().any(|report| report.param == limit.param))
        .collect()
}

/// The document with the rail's explorer link for its transaction, when the
/// document does not carry one, the rail has an explorer and the document
/// names a transaction.
fn with_explorer(network: &str, mut document: Value) -> Value {
    if document.is_object()
        && explorer(&document).is_none()
        && let Some(hash) = document
            .get(TRANSACTION)
            .and_then(Value::as_str)
            .filter(|hash| !hash.is_empty())
        && let Some(url) = explorer_link(network, LinkKind::Transaction, hash)
    {
        document[EXPLORER] = url.into();
    }
    document
}

/// Prints an `exec` or `status` result. JSON keeps the document and adds
/// `explorer`; text prints the same facts as `key  value` rows and one
/// table of limits. `payment` holds the per-payment limits of the installed
/// profile, when the caller holds it, as the SDK reads them. Such a limit has
/// no counter: JSON lists it under `paymentLimits` as `{param, limit}`, and
/// the table shows it without a spent or remaining amount. A document that
/// carries its own `paymentLimits` keeps them.
pub(crate) fn show(out: &mut Out, network: &str, document: Value, payment: &[PaymentLimit]) {
    let mut document = with_explorer(network, document);
    let own = WalletLimits::payment_of(&document);
    let payment = own.as_deref().unwrap_or(payment);
    if out.json {
        if document.is_object() && own.is_none() && !payment.is_empty() {
            document[WalletLimits::PAYMENT_KEY] = serde_json::to_value(payment).unwrap_or_default();
        }
        out.json_value(&document);
        return;
    }
    let (rows, limits) = rows(&document);
    out.stdout.push_str(&term::render(&rows));
    let limits = limits_table(&limits, payment);
    if !limits.is_empty() {
        out.line("");
        out.table(&limits);
    }
}

/// Prints the result of an owner workflow: a deployment, a funding or the
/// status of either. The SDK document names each transaction: its hash and,
/// once the ledger holds it, its ledger and status. JSON keeps the document
/// and adds the rail's explorer link to each transaction that has none.
/// Text prints the request first, then one block for each transaction, then
/// every other field.
pub(crate) fn show_owner(out: &mut Out, network: &str, mut document: Value) {
    for key in [RECEIPTS, TRANSACTIONS] {
        if let Some(items) = document.get_mut(key).and_then(Value::as_array_mut) {
            for item in items {
                *item = with_explorer(network, item.take());
            }
        }
    }
    if out.json {
        out.json_value(&document);
        return;
    }
    let Some(map) = document.as_object() else {
        out.text(&document);
        return;
    };
    let blocks = transaction_blocks(network, &document);
    let leading = ["requestId", "state", "network", "completed"];
    let mut rows = vec![];
    for key in leading {
        if let Some(value) = map.get(key) {
            term::flatten(key.into(), value, &mut rows);
        }
    }
    out.stdout.push_str(&term::render(&rows));
    for block in &blocks {
        out.line("");
        out.stdout.push_str(&term::render(block));
    }
    let mut rest = vec![];
    for (key, value) in map {
        let shown = leading.contains(&key.as_str())
            || (!blocks.is_empty() && [RECEIPTS, TRANSACTIONS].contains(&key.as_str()));
        if !shown {
            term::flatten(key.clone(), value, &mut rest);
        }
    }
    if !rest.is_empty() {
        out.line("");
        out.stdout.push_str(&term::render(&rest));
    }
}

/// One block of `key  value` rows for each transaction of an owner workflow
/// document. The verified receipts come first; a document without them
/// lists its transactions, which are objects or bare hashes.
fn transaction_blocks(network: &str, document: &Value) -> Vec<Vec<(String, String)>> {
    let list = |key: &str| {
        document
            .get(key)
            .and_then(Value::as_array)
            .filter(|items| !items.is_empty())
    };
    let Some(items) = list(RECEIPTS).or_else(|| list(TRANSACTIONS)) else {
        return vec![];
    };
    items
        .iter()
        .map(|item| {
            // A bare hash is a transaction that the document does not describe further.
            let item = match item {
                Value::String(hash) => {
                    with_explorer(network, serde_json::json!({ TRANSACTION: hash }))
                }
                other => other.clone(),
            };
            let mut rows = vec![];
            for key in ["action", TRANSACTION, "ledger", "status", EXPLORER] {
                if let Some(value) = item.get(key) {
                    term::flatten(key.into(), value, &mut rows);
                }
            }
            if let Some(map) = item.as_object() {
                for (key, value) in map {
                    if !rows.iter().any(|(shown, _)| shown == key) {
                        term::flatten(key.clone(), value, &mut rows);
                    }
                }
            } else {
                term::flatten(String::new(), &item, &mut rows);
            }
            rows
        })
        .collect()
}

/// The text rows of a result and its limit reports.
fn rows(document: &Value) -> (Vec<(String, String)>, Vec<LimitReport>) {
    let Some(map) = document.as_object() else {
        let mut rows = vec![];
        term::flatten(String::new(), document, &mut rows);
        return (rows, vec![]);
    };
    let settlement = Settlement::of(document);
    // The SDK reads the fee that the limits count and each limit report.
    let charged = fee_charged(document);
    let limits = Limits::of(document);
    let mut rows = vec![];
    // The request first, then what settled, then every other field.
    let leading = ["requestId", "status", TRANSACTION];
    for key in leading {
        if let Some(value) = map.get(key) {
            term::flatten(key.into(), value, &mut rows);
        }
    }
    if let Some(settlement) = &settlement {
        rows.push(("height".into(), settlement.height.to_string()));
        rows.push(("fee".into(), settlement.fee.text.clone()));
        if let Some(amount) = &settlement.amount {
            rows.push(("amount".into(), amount.text.clone()));
        }
    }
    if let Some(charged) = &charged {
        rows.push((FEE_CHARGED.into(), charged.text.clone()));
    }
    for (key, value) in map {
        let shown = leading.contains(&key.as_str())
            || (key == Settlement::KEY && settlement.is_some())
            || (key == FEE_CHARGED && charged.is_some())
            || (key == Limits::KEY && limits.is_some())
            || (key == WalletLimits::PAYMENT_KEY && WalletLimits::payment_of(document).is_some());
        if !shown {
            term::flatten(key.clone(), value, &mut rows);
        }
    }
    (rows, limits.unwrap_or_default())
}

/// The table of limits, header first; empty without a limit. `spent`
/// includes the fees that the limits count. A per-payment limit has no
/// counter, so its spent and remaining cells hold `-`: the table never
/// shows a zero that nothing counted.
pub(crate) fn limits_table(limits: &[LimitReport], payment: &[PaymentLimit]) -> Vec<Vec<String>> {
    let uncounted = uncounted(payment, limits);
    let mut table = vec![];
    if !limits.is_empty() || !uncounted.is_empty() {
        table.push(
            ["param", "limit", "spent", "remaining"]
                .map(String::from)
                .to_vec(),
        );
    }
    for limit in limits {
        table.push(vec![
            limit.param.clone(),
            limit.limit.text.clone(),
            limit.spent.text.clone(),
            limit.remaining.text.clone(),
        ]);
    }
    for limit in uncounted {
        table.push(vec![
            limit.param.clone(),
            limit.limit.text.clone(),
            "-".into(),
            "-".into(),
        ]);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use allowit_sdk::{execution::Quantity, kinds::AssetDisplay};
    use serde_json::json;

    fn asset() -> AssetDisplay {
        AssetDisplay {
            symbol: "XYZ".into(),
            decimals: 7,
            name: "Example".into(),
            native: true,
        }
    }
    fn quantity(units: i128) -> Value {
        serde_json::to_value(Quantity::new(units, asset())).unwrap()
    }
    fn settled() -> Value {
        json!({
            "requestId": "pay-0001",
            "status": "confirmed",
            "transactionHash": "ab".repeat(32),
            "settlement": {"height": 5_661_578, "fee": quantity(123_456), "amount": quantity(50_000_000)},
            "feeCharged": quantity(123_456),
            "limits": [
                {"param": "daily_spending_limit", "limit": quantity(100_000_000), "spent": quantity(50_123_456), "remaining": quantity(49_876_544)},
                {"param": "lifetime_spending_limit", "limit": quantity(1_000_000_000), "spent": quantity(50_123_456), "remaining": quantity(949_876_544)},
            ],
        })
    }
    fn text(document: Value) -> String {
        let mut out = Out::new(false, true, false, true);
        // No registered rail has this network, so no explorer link is added.
        show(&mut out, "example:none", document, &[]);
        out.stdout
    }

    #[test]
    fn a_settled_request_prints_its_height_fee_and_each_limit_with_units() {
        let hash = "ab".repeat(32);
        assert_eq!(
            text(settled()),
            format!(
                "requestId        pay-0001\n\
                 status           confirmed\n\
                 transactionHash  {hash}\n\
                 height           5661578\n\
                 fee              0.0123456 XYZ\n\
                 amount           5 XYZ\n\
                 feeCharged       0.0123456 XYZ\n\
                 \n\
                 param                    limit    spent          remaining\n\
                 daily_spending_limit     10 XYZ   5.0123456 XYZ  4.9876544 XYZ\n\
                 lifetime_spending_limit  100 XYZ  5.0123456 XYZ  94.9876544 XYZ\n"
            )
        );
    }

    #[test]
    fn a_pending_request_prints_only_what_the_document_holds() {
        let pending =
            json!({"requestId": "pay-0001", "status": "submitted", "transactionHash": ""});
        assert_eq!(
            text(pending),
            "requestId        pay-0001\nstatus           submitted\ntransactionHash  -\n"
        );
        // A field that is not a quantity prints as the document holds it.
        let odd = json!({"requestId": "pay-0001", "feeCharged": "7", "limits": [], "note": "x"});
        assert_eq!(
            text(odd),
            "requestId   pay-0001\nfeeCharged  7\nnote        x\n"
        );
    }

    #[test]
    fn json_keeps_the_document_and_adds_the_explorer_link_of_the_rail() {
        for rail in crate::routing::all() {
            let hash = "ab".repeat(32);
            let mut out = Out::new(true, true, false, true);
            show(&mut out, rail.id, settled(), &[]);
            let printed: Value = serde_json::from_str(&out.stdout).unwrap();
            let mut expected = settled();
            if let Some(url) = explorer_link(rail.id, LinkKind::Transaction, &hash) {
                assert!(url.ends_with(&hash));
                expected[EXPLORER] = url.into();
            }
            assert_eq!(printed, expected);
            // A request without a transaction gets no link.
            let mut out = Out::new(true, true, false, true);
            let pending = json!({"requestId": "pay-0001", "status": "prepared"});
            show(&mut out, rail.id, pending.clone(), &[]);
            assert_eq!(serde_json::from_str::<Value>(&out.stdout).unwrap(), pending);
        }
    }

    fn payment(units: i128) -> PaymentLimit {
        PaymentLimit {
            param: "payment_limit".into(),
            limit: serde_json::from_value(quantity(units)).unwrap(),
        }
    }

    #[test]
    fn a_limit_of_each_payment_shows_without_counters() {
        let limits = [payment(10_000_000)];
        // Text: the limit is a row of the table, and nothing counts it.
        let mut out = Out::new(false, true, false, true);
        show(&mut out, "example:none", settled(), &limits);
        assert!(
            out.stdout.ends_with(
                "param                    limit    spent          remaining\n\
                 daily_spending_limit     10 XYZ   5.0123456 XYZ  4.9876544 XYZ\n\
                 lifetime_spending_limit  100 XYZ  5.0123456 XYZ  94.9876544 XYZ\n\
                 payment_limit            1 XYZ    -              -\n"
            ),
            "{}",
            out.stdout
        );
        // JSON: the reports stay as the SDK types them; the limit has the SDK's own key and shape.
        let mut out = Out::new(true, true, false, true);
        show(&mut out, "example:none", settled(), &limits);
        let printed: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(printed[Limits::KEY], settled()[Limits::KEY]);
        assert_eq!(
            printed[WalletLimits::PAYMENT_KEY],
            json!([{"param": "payment_limit", "limit": quantity(10_000_000)}])
        );
        assert_eq!(WalletLimits::payment_of(&printed), Some(limits.to_vec()));
        assert_eq!(Limits::of(&printed), Limits::of(&settled()));
        // A request without a report still shows the limit, and no zero.
        let pending = json!({"requestId": "pay-0001", "status": "submitted"});
        let mut out = Out::new(false, true, false, true);
        show(&mut out, "example:none", pending.clone(), &limits);
        assert_eq!(
            out.stdout,
            "requestId  pay-0001\nstatus     submitted\n\n\
             param          limit  spent  remaining\n\
             payment_limit  1 XYZ  -      -\n"
        );
        // A document that carries its own per-payment limits keeps them, in text and in JSON.
        let mut own = pending.clone();
        own[WalletLimits::PAYMENT_KEY] = json!([payment(20_000_000)]);
        let mut out = Out::new(false, true, false, true);
        show(&mut out, "example:none", own.clone(), &limits);
        assert!(
            out.stdout.ends_with("payment_limit  2 XYZ  -      -\n"),
            "{}",
            out.stdout
        );
        assert!(!out.stdout.contains("paymentLimits"), "{}", out.stdout);
        let mut out = Out::new(true, true, false, true);
        show(&mut out, "example:none", own.clone(), &limits);
        assert_eq!(serde_json::from_str::<Value>(&out.stdout).unwrap(), own);
        // A limit that the rail reports with counters is not repeated, and a
        // profile without the param adds nothing.
        let counted = LimitReport {
            param: "payment_limit".into(),
            limit: serde_json::from_value(quantity(1)).unwrap(),
            spent: serde_json::from_value(quantity(0)).unwrap(),
            remaining: serde_json::from_value(quantity(1)).unwrap(),
        };
        assert_eq!(limits_table(&[counted], &limits).len(), 2);
        assert!(limits_table(&[], &[]).is_empty());
        let mut out = Out::new(true, true, false, true);
        show(&mut out, "example:none", pending.clone(), &[]);
        assert_eq!(serde_json::from_str::<Value>(&out.stdout).unwrap(), pending);
    }

    fn deployed(hash: &str) -> Value {
        json!({
            "requestId": "deploy-0001",
            "state": "installed",
            "selection": {"config": {"wallet": "WALLET"}},
            "transactions": [hash],
            "receipts": [{"action": "deploy_wallet", "transactionHash": hash, "status": "confirmed", "ledger": 5_682_550}],
        })
    }

    #[test]
    fn an_owner_workflow_prints_the_hash_ledger_and_status_of_each_transaction() {
        let hash = "cd".repeat(32);
        let mut out = Out::new(false, true, false, true);
        show_owner(&mut out, "example:none", deployed(&hash));
        assert_eq!(
            out.stdout,
            format!(
                "requestId  deploy-0001\n\
                 state      installed\n\
                 \n\
                 action           deploy_wallet\n\
                 transactionHash  {hash}\n\
                 ledger           5682550\n\
                 status           confirmed\n\
                 \n\
                 selection.config.wallet  WALLET\n"
            )
        );
        // A status document describes each transaction itself; one that the
        // ledger does not hold yet has no ledger.
        let status = json!({
            "requestId": "fund-0001", "network": "example:none", "completed": false,
            "transactions": [{"action": "fund_wallet", "transactionHash": hash, "status": "submissionUnknown", "ledger": null}],
        });
        let mut out = Out::new(false, true, false, true);
        show_owner(&mut out, "example:none", status);
        assert_eq!(
            out.stdout,
            format!(
                "requestId  fund-0001\n\
                 network    example:none\n\
                 completed  false\n\
                 \n\
                 action           fund_wallet\n\
                 transactionHash  {hash}\n\
                 ledger           -\n\
                 status           submissionUnknown\n"
            )
        );
        // A document that names only hashes prints them, and one without a
        // transaction prints as every other document does.
        let mut bare = deployed(&hash);
        bare.as_object_mut().unwrap().remove("receipts");
        let mut out = Out::new(false, true, false, true);
        show_owner(&mut out, "example:none", bare);
        assert!(
            out.stdout
                .contains(&format!("\n\ntransactionHash  {hash}\n\n")),
            "{}",
            out.stdout
        );
        let mut out = Out::new(false, true, false, true);
        show_owner(
            &mut out,
            "example:none",
            json!({"requestId": "activate-0001", "state": "active", "transactions": []}),
        );
        assert_eq!(
            out.stdout,
            "requestId  activate-0001\nstate      active\n\ntransactions  -\n"
        );
    }

    #[test]
    fn an_owner_workflow_gets_the_explorer_link_of_the_rail_for_each_transaction() {
        for rail in crate::routing::all() {
            let hash = "cd".repeat(32);
            let Some(url) = explorer_link(rail.id, LinkKind::Transaction, &hash) else {
                continue;
            };
            // JSON keeps the document, so the next owner command reads it,
            // and each receipt gains the link.
            let mut out = Out::new(true, true, false, true);
            show_owner(&mut out, rail.id, deployed(&hash));
            let printed: Value = serde_json::from_str(&out.stdout).unwrap();
            let mut expected = deployed(&hash);
            expected["receipts"][0][EXPLORER] = url.clone().into();
            assert_eq!(printed, expected);
            // A link that the document carries is kept.
            let mut linked = deployed(&hash);
            linked["receipts"][0][EXPLORER] = "https://example.invalid/tx".into();
            let mut out = Out::new(true, true, false, true);
            show_owner(&mut out, rail.id, linked.clone());
            assert_eq!(serde_json::from_str::<Value>(&out.stdout).unwrap(), linked);
            // Text prints the link with the transaction, also for a bare hash.
            let mut out = Out::new(false, true, false, true);
            show_owner(&mut out, rail.id, deployed(&hash));
            assert!(out.stdout.contains(&format!("explorer         {url}\n")));
            let mut bare = deployed(&hash);
            bare.as_object_mut().unwrap().remove("receipts");
            let mut out = Out::new(false, true, false, true);
            show_owner(&mut out, rail.id, bare);
            assert!(out.stdout.contains(&format!("explorer         {url}\n")));
        }
    }

    #[test]
    fn no_amount_prints_a_placeholder_unit() {
        let shown = text(settled());
        assert!(!shown.contains("UNIT"));
        for line in shown.lines().filter(|line| line.contains("XYZ")) {
            assert!(line.split("  ").any(|cell| cell.trim().ends_with(" XYZ")));
        }
    }
}
