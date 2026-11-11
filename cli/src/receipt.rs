//! The receipt of an agent request: what settled, what it cost and what
//! remains of each spending limit. Every amount is a quantity that carries
//! its own unit text, from the SDK or the server; this file never names a unit.
use crate::term::{self, Out};
use allowit_sdk::{
    execution::{
        EXPLORER_KEY as EXPLORER, FEE_CHARGED_KEY as FEE_CHARGED, LimitReport, Limits, Settlement,
        explorer, fee_charged,
    },
    runtime::owner::{LinkKind, explorer_link},
};
use serde_json::Value;

const TRANSACTION: &str = "transactionHash";

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
/// table of limits.
pub(crate) fn show(out: &mut Out, network: &str, document: Value) {
    let document = with_explorer(network, document);
    if out.json {
        out.json_value(&document);
        return;
    }
    let (rows, limits) = rows(&document);
    out.stdout.push_str(&term::render(&rows));
    if !limits.is_empty() {
        out.line("");
        out.table(&limits);
    }
}

/// The text rows of a result and its table of limits, header first.
fn rows(document: &Value) -> (Vec<(String, String)>, Vec<Vec<String>>) {
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
            || (key == Limits::KEY && limits.is_some());
        if !shown {
            term::flatten(key.clone(), value, &mut rows);
        }
    }
    (rows, limits_table(&limits.unwrap_or_default()))
}

/// The table of spending limits, header first; empty without a limit.
/// `spent` includes the fees that the limits count.
pub(crate) fn limits_table(limits: &[LimitReport]) -> Vec<Vec<String>> {
    let mut table = vec![];
    if !limits.is_empty() {
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
        show(&mut out, "example:none", document);
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
            show(&mut out, rail.id, settled());
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
            show(&mut out, rail.id, pending.clone());
            assert_eq!(serde_json::from_str::<Value>(&out.stdout).unwrap(), pending);
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
