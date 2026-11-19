//! Credits: what Allowit charges the owner of a policy for each request. The
//! owner pays for compute, whatever the outcome: one credit for the request,
//! the model tokens that it used, and the fees that Allowit paid for it, such
//! as the network fee of a settlement. The sum is rounded down once, to a
//! whole credit.
//!
//! The agent holds no credits and needs none. This module reads what the
//! evaluation of one request cost, for the receipt and the status, and it
//! explains the refusals that name credits: what happened and what the owner
//! must do. Every server text is checked before it is printed.
use crate::{agent_read, error::Error};
use allowit_sdk::wallet::LocalSigner;
use serde_json::{Value, json};
use std::{rc::Rc, time::Duration};

/// The key of the charge in a receipt, a status and a JSON error.
pub(crate) const KEY: &str = "credits";

/// The server refused before the evaluation: the owner cannot pay one.
pub(crate) const EXHAUSTED: &str = "credits_exhausted";
/// The evaluation stopped before a metered call that the owner's credits do
/// not cover. The request is denied.
#[cfg(test)]
pub(crate) const INSUFFICIENT: &str = "credits_insufficient";
/// The policy's analysis allowance does not cover a metered call today.
#[cfg(test)]
pub(crate) const ALLOWANCE: &str = "allowance_exhausted";
/// The server is evaluating the same request now.
pub(crate) const RUNNING: &str = "evaluation_running";
/// The server's credit ledger did not answer, so the server released no decision.
pub(crate) const UNAVAILABLE: &str = "credits_unavailable";

/// What one request cost its policy owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Charge {
    /// The credits charged, as a decimal.
    pub charged: String,
    /// The version of the price list that priced the request.
    pub tariff: String,
    /// Each item of the charge.
    pub lines: Vec<Line>,
}

/// One item of a charge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub item: String,
    /// What the item adds to the charge.
    pub credits: String,
    /// The exact cost of a call that is priced by its tokens, or of a fee, in
    /// credits. The item `inference` charges the whole credits of those
    /// costs together.
    pub cost: Option<String>,
    /// A fee as it was paid in an asset, such as a network fee.
    pub fee: Option<Fee>,
}

/// A fee as its payer paid it: the amount in its asset, the US dollar value
/// of one unit that the price list took, and the transaction that paid it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fee {
    pub asset: String,
    pub amount: String,
    pub usd_per_asset: String,
    pub transaction: Option<String>,
}

impl Fee {
    /// The fee in a server document. Any text that is not a plain amount or
    /// word is no fee.
    fn of(value: &Value) -> Option<Self> {
        Some(Self {
            asset: word(value["asset"].as_str()?, 12)?,
            amount: amount(value["amount"].as_str()?)?,
            usd_per_asset: amount(value["usdPerAsset"].as_str()?)?,
            transaction: match &value["transaction"] {
                Value::Null => None,
                transaction => Some(word(transaction.as_str()?, 128)?),
            },
        })
    }
    fn document(&self) -> Value {
        let mut fee = json!({
            "asset": self.asset,
            "amount": self.amount,
            "usdPerAsset": self.usd_per_asset,
        });
        if let Some(transaction) = &self.transaction {
            fee["transaction"] = json!(transaction);
        }
        fee
    }
}

/// The items that are fees that Allowit paid for the request.
const FEES: [&str; 3] = ["search_fee", "network_fee", "facilitator_fee"];

impl Line {
    /// The item as the receipt names it.
    fn text(&self) -> String {
        let name = match self.item.as_str() {
            "base" => "request".to_owned(),
            item => item.replace('_', " "),
        };
        match (&self.cost, &self.fee) {
            (Some(cost), Some(fee)) => format!(
                "{name} {cost} ({} {} at USD {})",
                fee.amount, fee.asset, fee.usd_per_asset
            ),
            (Some(_), None) if self.item == "inference" => format!("{name} {}", self.credits),
            (Some(cost), None) if FEES.contains(&self.item.as_str()) => format!("{name} {cost}"),
            (Some(cost), None) => format!("{name} {cost} used"),
            (None, _) => format!("{name} {}", self.credits),
        }
    }
}

/// A decimal amount of credits: digits, at most one point, at most 24 bytes.
fn amount(text: &str) -> Option<String> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, "0"));
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    (text.len() <= 24 && digits(whole) && digits(fraction)).then(|| text.to_owned())
}

/// A server name as a plain word: letters, digits, `_`, `-` and `.`.
fn word(text: &str, limit: usize) -> Option<String> {
    (!text.is_empty()
        && text.len() <= limit
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b)))
    .then(|| text.to_owned())
}

impl Charge {
    /// The charge in a server document `{charged, tariff, lines}`. A document
    /// with any text that is not a plain amount or word is no charge.
    pub(crate) fn of(value: &Value) -> Option<Self> {
        let lines = value["lines"]
            .as_array()?
            .iter()
            .take(64)
            .map(|line| {
                Some(Line {
                    item: word(line["item"].as_str()?, 32)?,
                    credits: amount(line["credits"].as_str()?)?,
                    cost: match &line["cost"] {
                        Value::Null => None,
                        cost => Some(amount(cost.as_str()?)?),
                    },
                    fee: match &line["fee"] {
                        Value::Null => None,
                        fee => Some(Fee::of(fee)?),
                    },
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            charged: amount(value["charged"].as_str()?)?,
            tariff: word(value["tariff"].as_str()?, 32)?,
            lines,
        })
    }
    /// The charge as JSON output carries it.
    pub(crate) fn document(&self) -> Value {
        json!({
            "charged": self.charged,
            "tariff": self.tariff,
            "lines": self.lines.iter().map(|line| {
                let mut item = json!({"item": line.item, "credits": line.credits});
                if let Some(cost) = &line.cost {
                    item["cost"] = json!(cost);
                }
                if let Some(fee) = &line.fee {
                    item["fee"] = fee.document();
                }
                item
            }).collect::<Vec<_>>(),
            "paidBy": "owner",
        })
    }
    /// One text row: the credits, each item and who paid. A call that is
    /// priced by its tokens shows what it used, a fee shows its cost and how
    /// it was paid, and `inference` is what the request paid for them.
    pub(crate) fn text(&self) -> String {
        let items: Vec<String> = self.lines.iter().map(Line::text).collect();
        let items = if items.len() > 1 {
            format!(" ({})", items.join(", "))
        } else {
            String::new()
        };
        format!(
            "{}{items}, charged to the policy owner, price list {}",
            self.charged, self.tariff
        )
    }
}

/// Where the charge of a request is read: the server, the policy and the
/// agent key that signs the read.
#[derive(Clone, Default)]
pub(crate) struct Lookup {
    origin: Option<String>,
    policy: Option<String>,
    signer: Option<Rc<LocalSigner>>,
}

impl Lookup {
    /// `policy` is the policy of a binding: the policy id, or `id@module digest`.
    /// `signer` is the agent key: the server answers the charge to the policy's
    /// agent only.
    pub(crate) fn new(
        origin: Option<String>,
        policy: Option<String>,
        signer: Option<LocalSigner>,
    ) -> Self {
        let policy = policy.map(|policy| match policy.split_once('@') {
            Some((id, _)) => id.to_owned(),
            None => policy,
        });
        Self {
            origin,
            policy,
            signer: signer.map(Rc::new),
        }
    }
    /// The charge of `request`, when the server holds one. The agent key signs
    /// the read, and a server that does not answer it leaves the result of the
    /// command as it is.
    pub(crate) fn read(&self, request: &str) -> Option<Charge> {
        let origin = crate::config::parse_origin(self.origin.as_deref()?).ok()?;
        let policy = self.policy.as_deref()?;
        let signer = self.signer.as_deref()?;
        if !crate::commands::policy_help::is_policy_id(policy)
            || !crate::request::request_id_valid(request)
        {
            return None;
        }
        let (status, body) = agent_read::post(
            &origin,
            agent_read::CHARGE_PATH,
            &json!({"policyId": policy, "requestId": request}),
            signer,
            agent_read::Limits {
                timeout: Duration::from_secs(5),
                connect: Duration::from_secs(3),
                max_bytes: 64 << 10,
            },
        )
        .ok()?;
        if status != 200 {
            return None;
        }
        let body: Value = serde_json::from_slice(&body).ok()?;
        Charge::of(&body[KEY])
    }
    /// `document` with the charge of `request`, when the server holds one.
    pub(crate) fn attach(&self, document: &mut Value, request: &str) {
        if document.is_object()
            && let Some(charge) = self.read(request)
        {
            document[KEY] = charge.document();
        }
    }
}

/// True for a refusal of a request that Allowit may have charged. The owner
/// pays for each request that Allowit evaluates, whatever its outcome: a
/// denial of the policy, of the server's policy runner or of a decision
/// question, a request that waits for its owner, a metered call that the
/// credits or the allowance did not cover, and a step that failed. The
/// refusals before the evaluation, and a ledger that did not answer,
/// charged nothing. The charge is then read from the server, which answers
/// only when it holds one.
pub(crate) fn charged_denial(error: &Error) -> bool {
    error.code == crate::exit::Exit::Denied.code()
        // A refusal of the server: a check of this CLI before signing has no status and was
        // never evaluated.
        || (error.status.is_some()
            && error
                .api_code
                .as_deref()
                .is_some_and(|code| !matches!(code, EXHAUSTED | RUNNING | UNAVAILABLE)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_charge_reads_only_plain_amounts_and_words() {
        let charge = Charge::of(&json!({
            "charged": "37.0", "tariff": "2026-11-17",
            "lines": [
                {"item": "base", "credits": "1.0"},
                {"item": "decision_api", "credits": "0.0", "cost": "0.24", "inputTokens": 2400, "usdNanos": 240000},
                {"item": "web_search", "credits": "0.0", "cost": "0.912", "searches": 1},
                {"item": "search_fee", "credits": "0.0", "cost": "35.0", "count": 1},
                {"item": "fetch", "credits": "0.0"},
                {"item": "inference", "credits": "36.0", "cost": "36.152"},
            ],
        }))
        .unwrap();
        assert_eq!(
            charge.text(),
            "37.0 (request 1.0, decision api 0.24 used, web search 0.912 used, search fee 35.0, fetch 0.0, inference 36.0), charged to the policy owner, price list 2026-11-17"
        );
        assert_eq!(charge.document()["paidBy"], "owner");
        assert_eq!(charge.document()["lines"][1]["item"], "decision_api");
        assert_eq!(charge.document()["lines"][1]["cost"], "0.24");
        assert!(charge.document()["lines"][0].get("cost").is_none());
        // An exact use shows every digit that the server sent.
        let small = Charge::of(&json!({"charged": "1.0", "tariff": "v1", "lines": [
            {"item": "base", "credits": "1.0"},
            {"item": "decision_api", "credits": "0.0", "cost": "0.000012"},
            {"item": "inference", "credits": "0.0", "cost": "0.000012"},
        ]}))
        .unwrap();
        assert_eq!(
            small.text(),
            "1.0 (request 1.0, decision api 0.000012 used, inference 0.0), charged to the policy owner, price list v1"
        );
        // A charge of the first price list still reads.
        let first = Charge::of(&json!({"charged": "1.1", "tariff": "2026-11-14", "lines": [
            {"item": "base", "credits": "1.0"}, {"item": "decision_api", "credits": "0.1", "usdNanos": 29000},
        ]}))
        .unwrap();
        assert_eq!(
            first.text(),
            "1.1 (request 1.0, decision api 0.1), charged to the policy owner, price list 2026-11-14"
        );
        // A network fee that Allowit paid after the decision is a later entry of
        // the same request: its line shows the fee in its asset and the rate.
        let settled = Charge::of(&json!({"charged": "2.0", "tariff": "2026-11-17.2", "lines": [
            {"item": "base", "credits": "1.0"},
            {"item": "decision_api", "credits": "0.0", "cost": "0.9"},
            {"item": "inference", "credits": "0.0", "cost": "0.9"},
            {"item": "network_fee", "credits": "0.0", "cost": "0.156156", "usdNanos": 156156, "fee": {
                "asset": "XYZ", "units": "11154", "amount": "0.0011154", "usdPerAsset": "0.14",
                "network": "net:test", "transaction": "ab12",
            }},
            {"item": "inference", "credits": "1.0", "cost": "0.156156", "carried": "0.9"},
        ]}))
        .unwrap();
        assert_eq!(
            settled.text(),
            "2.0 (request 1.0, decision api 0.9 used, inference 0.0, network fee 0.156156 (0.0011154 XYZ at USD 0.14), inference 1.0), charged to the policy owner, price list 2026-11-17.2"
        );
        assert_eq!(
            settled.document()["lines"][3]["fee"],
            json!({"asset": "XYZ", "amount": "0.0011154", "usdPerAsset": "0.14", "transaction": "ab12"})
        );
        let simple = Charge::of(&json!({"charged": "1.0", "tariff": "v1", "lines": [{"item": "base", "credits": "1.0"}]})).unwrap();
        assert_eq!(
            simple.text(),
            "1.0, charged to the policy owner, price list v1"
        );
        // Server text never reaches the terminal unchecked.
        for bad in [
            json!({"charged": "1.0\u{1b}[31m", "tariff": "v1", "lines": []}),
            json!({"charged": "-1.0", "tariff": "v1", "lines": []}),
            json!({"charged": "1e3", "tariff": "v1", "lines": []}),
            json!({"charged": 1.0, "tariff": "v1", "lines": []}),
            json!({"charged": "1.0", "tariff": "v1 $(rm)", "lines": []}),
            json!({"charged": "1.0", "tariff": "v1", "lines": [{"item": "base\nx", "credits": "1.0"}]}),
            json!({"charged": "1.0", "tariff": "v1", "lines": [{"item": "base", "credits": "one"}]}),
            json!({"charged": "1.0", "tariff": "v1", "lines": [{"item": "model", "credits": "0.0", "cost": "0.1\u{1b}[31m"}]}),
            json!({"charged": "1.0", "tariff": "v1", "lines": [{"item": "model", "credits": "0.0", "cost": 0.1}]}),
            json!({"charged": "1.0", "tariff": "v1", "lines": [{"item": "network_fee", "credits": "0.0", "cost": "0.1", "fee": {"asset": "XYZ\u{1b}", "amount": "0.1", "usdPerAsset": "0.14"}}]}),
            json!({"charged": "1.0", "tariff": "v1", "lines": [{"item": "network_fee", "credits": "0.0", "cost": "0.1", "fee": {"asset": "XYZ", "amount": "0.1", "usdPerAsset": "0.14", "transaction": "a b"}}]}),
            json!({"charged": "1.0", "tariff": "v1", "lines": [{"item": "network_fee", "credits": "0.0", "cost": "0.1", "fee": {"asset": "XYZ", "amount": "0.1"}}]}),
            json!({"charged": "1.0", "tariff": "v1"}),
            json!(null),
        ] {
            assert!(Charge::of(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn every_charged_denial_reads_its_charge() {
        let mut policy = Error::api(409, "The policy denied this operation");
        policy.code = 20;
        assert!(charged_denial(&policy));
        // The server's policy runner and a decision question deny with their own codes. A
        // request that waits for its owner, and one whose step failed, were evaluated too.
        for (status, code) in [
            (403, "server_policy_denied"),
            (403, "decision_denied"),
            (409, INSUFFICIENT),
            (409, ALLOWANCE),
            (409, "owner_review_pending"),
            (409, "wallet_rejected"),
        ] {
            assert!(charged_denial(&Error::refused(status, code, "x")), "{code}");
        }
        // A refusal before the evaluation, and a ledger that did not answer, charged nothing.
        for (status, code) in [(402, EXHAUSTED), (409, RUNNING), (503, UNAVAILABLE)] {
            assert!(
                !charged_denial(&Error::refused(status, code, "x")),
                "{code}"
            );
        }
        assert!(!charged_denial(&Error::uncertain("no answer")));
        // A check of this CLI before signing: typed, with no server status, never evaluated.
        let mut local = Error::config("Nothing was signed.");
        local.code = 4;
        local.api_code = Some("server_key_differs".into());
        assert!(!charged_denial(&local));
    }

    #[test]
    fn a_lookup_without_a_server_or_a_policy_reads_nothing() {
        assert!(Lookup::default().read("pay-0001").is_none());
        let lookup = Lookup::new(
            Some("http://127.0.0.1:9".into()),
            Some("not a policy id".into()),
            None,
        );
        assert!(lookup.read("pay-0001").is_none());
        let lookup = Lookup::new(
            Some("http://127.0.0.1:9".into()),
            Some("Xg7IxRXCr2MhVdRrTFoYKeAw".into()),
            None,
        );
        assert!(lookup.read("bad id").is_none());
        // Without the agent key there is no read: the server answers the agent only.
        assert!(lookup.read("pay-0001").is_none());
        // A binding names its policy with the module digest: the lookup asks by the policy id.
        let bound = Lookup::new(None, Some("Xg7IxRXCr2MhVdRrTFoYKeAw@0123abcd".into()), None);
        assert_eq!(bound.policy.as_deref(), Some("Xg7IxRXCr2MhVdRrTFoYKeAw"));
    }
}
