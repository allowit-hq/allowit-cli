//! What a decision question leaves to rules: an amount, an address, a price or a date.
//!
//! A decision model passes a mismatch of such a detail with a high score, such as ten times an
//! invoice, another vault, twice a listed price or a refund of an order older than its window.
//! A rule checks each one exactly. `allowit policy check` warns when a question that the source
//! states as a literal asks one (see [`warnings`]); the server's drafting applies the same list.
//!
//! The classification is a fixed list of English patterns over the question text, matched
//! without regard to case:
//!
//! - Amount: a number with a currency code, word or sign (`10 USDC`, `$5`, `20 dollars`); a
//!   comparison word before a number (`at most 50`, `more than 3`, `exceeds 100`); a percentage;
//!   a determiner or qualifier before `amount` (`the amount`, `exact amount`, `invoiced amount`).
//! - Address: a Stellar account or contract key; a determiner or qualifier directly before
//!   `address` (`the address`, `listed address`, `vendor's address`), except the verb after a
//!   demonstrative, as in `does this address the vault`.
//! - Price: a qualifier before `price` (`the price`, `listed price`, `share price`); `exchange
//!   rate` or `conversion rate`; `cheaper than`, `more expensive than`, `costs more`.
//! - Date: an ISO date; a window such as `within the last 30 days` or `in 2 weeks`; an age such
//!   as `45 days old` or `3 days ago`; `older than`, `earlier than`, `later than`; a month name
//!   after `before`, `after`, `until`, `by`, `since` or `on`; `expired`, `expires`, `expiry`,
//!   `deadline`, `due date`, `overdue`; `today`, `yesterday`, `tomorrow`.
//!
//! It does not catch a detail that the question implies without these words, such as "Is the
//! payment too large?", "Does it match the invoice?" or "Is this paid to Ana?"; numbers and
//! dates in words without a unit ("ten dollars" is caught, "ten" alone is not); a question that
//! the policy reads from a param, such as `query: question.as_str()`; and text in other
//! languages. It can flag a question that uses these words for something else, such as "Is this
//! payment for the price list of a competitor?".
use regex::Regex;
use std::sync::OnceLock;

/// A detail that a rule checks better than a decision question.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Detail {
    Amount,
    Address,
    Price,
    Date,
}

impl Detail {
    /// The detail with its article, as a diagnostic names it.
    pub(crate) const fn noun(self) -> &'static str {
        match self {
            Self::Amount => "an amount",
            Self::Address => "an address",
            Self::Price => "a price",
            Self::Date => "a date",
        }
    }

    /// The rule that checks this detail, as a warning names it.
    pub(crate) const fn rule(self) -> &'static str {
        match self {
            Self::Amount => {
                "payment_limit, daily_spending_limit or custom code that compares ctx.request.amount with an amount param"
            }
            Self::Address => "destinations with the addresses",
            Self::Price => {
                "payment_limit or custom code that compares ctx.request.amount with an amount param, because the price that the agent pays is the payment's amount"
            }
            Self::Date => {
                "expires_at, or custom code that compares ctx.request.now with a time param"
            }
        }
    }
}

/// One detail that a question asks, and the words that ask it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub(crate) detail: Detail,
    pub(crate) words: String,
}

/// Demonstratives: before `address` and a determiner they make it the verb, as in "does this
/// address the vault".
const DEMONSTRATIVES: [&str; 4] = ["this", "that", "these", "those"];
/// Words that start a noun phrase.
const DETERMINERS: [&str; 12] = [
    "the", "a", "an", "this", "that", "these", "those", "its", "their", "our", "your", "my",
];
const MONTHS: &str = "january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sep|sept|oct|nov|dec";
const UNITS: &str = "seconds?|minutes?|hours?|days?|weeks?|months?|years?";

fn patterns() -> &'static [(Detail, Regex)] {
    static PATTERNS: OnceLock<Vec<(Detail, Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let qualifier = r"the|a|an|this|that|its|their|same|exact|right|correct|expected|stated|listed|approved|agreed|invoiced|invoice|registered|known|wallet|payee|recipient|destination|sender|payer|vendor'?s?|person'?s?|owner'?s?|seller'?s?";
        let list: Vec<(Detail, String)> = vec![
            (
                Detail::Amount,
                r"\d[\d,]*(?:\.\d+)?\s*(?:usdc|eurc|xlm|usd|eur|gbp|dollars?|euros?|cents?)\b"
                    .into(),
            ),
            (Detail::Amount, r"[$€£]\s?\d".into()),
            (
                Detail::Amount,
                r"\b(?:at most|at least|no more than|no less than|more than|less than|fewer than|up to|over|under|above|below|exceeds?|exceeding|greater than|smaller than|larger than)\s+[$€£]?\d"
                    .into(),
            ),
            (Detail::Amount, r"\d(?:\.\d+)?\s*(?:%|percent\b)".into()),
            (
                Detail::Amount,
                format!(r"\b(?:{qualifier}|in the)\s+(?:payment\s+)?amounts?\b"),
            ),
            (Detail::Address, r"\b[GC][A-Z2-7]{55}\b".into()),
            (
                Detail::Address,
                format!(r"\b({qualifier})\s+address(?:es)?\b(?:\s+(\w+))?"),
            ),
            (
                Detail::Price,
                r"\b(?:the|this|that|its|their|same|listed|stated|quoted|agreed|reference|market|share|current|expected|fair|unit|asking|sale|list)\s+prices?\b"
                    .into(),
            ),
            (Detail::Price, r"\b(?:exchange|conversion)\s+rates?\b".into()),
            (
                Detail::Price,
                r"\b(?:cheaper than|more expensive than|costs? (?:more|less|at most|at least|no more|over|under))\b"
                    .into(),
            ),
            (Detail::Date, r"\b\d{4}-\d{2}-\d{2}\b".into()),
            (
                Detail::Date,
                format!(
                    r"\b(?:within|in|during|over)\s+(?:the\s+)?(?:last|past|next|previous|first)?\s*(?:\d+|\w+)\s+(?:{UNITS})\b"
                ),
            ),
            (
                Detail::Date,
                format!(r"\b\d+\s*-?\s*(?:{UNITS})\s+(?:old|ago|before|after|earlier|later)\b"),
            ),
            (
                Detail::Date,
                r"\b(?:older|newer|earlier|later|sooner)\s+than\b".into(),
            ),
            (
                Detail::Date,
                format!(
                    r"\b(?:before|after|until|by|since|on)\s+(?:the\s+)?(?:\d{{1,2}}(?:st|nd|rd|th)?\s+(?:of\s+)?)?(?:{MONTHS})\b"
                ),
            ),
            (
                Detail::Date,
                r"\b(?:expired|expires|expiry|expiration|deadline|due date|overdue|today|yesterday|tomorrow)\b"
                    .into(),
            ),
        ];
        list.into_iter()
            .map(|(detail, pattern)| {
                let regex = Regex::new(&format!("(?i){pattern}"))
                    .expect("the detail patterns are valid");
                (detail, regex)
            })
            .collect()
    })
}

/// The details that `question` asks, each once, with the first words that ask it, in the order
/// amount, address, price, date.
pub(crate) fn classify(question: &str) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    for (detail, pattern) in patterns() {
        if found.iter().any(|f| f.detail == *detail) {
            continue;
        }
        // Only the address pattern captures: its qualifier, and the word after `address`.
        let word = |capture: &regex::Captures, group: usize, list: &[&str]| {
            capture
                .get(group)
                .is_some_and(|word| list.contains(&word.as_str().to_ascii_lowercase().as_str()))
        };
        let hit = pattern
            .captures_iter(question)
            .find(|capture| !(word(capture, 1, &DEMONSTRATIVES) && word(capture, 2, &DETERMINERS)));
        if let Some(capture) = hit {
            let whole = capture.get(0).expect("a match has its whole text");
            // The words without the word after `address`.
            let end = capture.get(2).map_or(whole.end(), |next| next.start());
            let words = question[whole.start()..end].trim_end();
            found.push(Found {
                detail: *detail,
                words: words.to_owned(),
            });
        }
    }
    found.sort_by_key(|f| f.detail);
    found
}

/// The questions that `source` states as literals, collected from its syntax tree.
struct Questions(Vec<String>);

impl<'ast> syn::visit::Visit<'ast> for Questions {
    fn visit_expr_struct(&mut self, node: &'ast syn::ExprStruct) {
        let asks = node.path.segments.last().is_some_and(|segment| {
            segment.ident == "AllowitDecision" || segment.ident == "AllowitRequestScreening"
        });
        if asks {
            for field in &node.fields {
                if matches!(&field.member, syn::Member::Named(name) if name == "query")
                    && let Some(text) = literal(&field.expr)
                    && !self.0.contains(&text)
                {
                    self.0.push(text);
                }
            }
        }
        syn::visit::visit_expr_struct(self, node);
    }
}

/// The text of a string literal, ordinary or raw, also in parentheses.
fn literal(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(text),
            ..
        }) => Some(text.value()),
        syn::Expr::Paren(inner) => literal(&inner.expr),
        syn::Expr::Group(inner) => literal(&inner.expr),
        _ => None,
    }
}

/// The literal questions of `source`: the `query` field of each `AllowitDecision` and
/// `AllowitRequestScreening` struct expression whose value is a string literal, ordinary or
/// raw, as Rust parses the source. Comments, other fields and a question that a variable holds
/// do not count. A source that does not parse, or whose nesting the shared source-shape check
/// refuses, has none: the policy gate refuses such a source before this runs.
pub(crate) fn questions(source: &str) -> Vec<String> {
    if allowit_sdk::parameter_reads::check_source_shape(source).is_err() {
        return Vec::new();
    }
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let mut found = Questions(Vec::new());
    syn::visit::Visit::visit_file(&mut found, &file);
    found.0
}

/// One warning for each literal question of `source` that asks a detail.
pub(crate) fn warnings(source: &str) -> Vec<String> {
    questions(source)
        .iter()
        .filter_map(|question| {
            let found = classify(question);
            if found.is_empty() {
                return None;
            }
            let asked: Vec<String> = found
                .iter()
                .map(|f| format!("{} (\"{}\")", f.detail.noun(), f.words))
                .collect();
            let rules: Vec<String> = found
                .iter()
                .map(|f| format!("for {}, {}", f.detail.noun(), f.detail.rule()))
                .collect();
            Some(format!(
                "the decision question \"{question}\" asks {}; a decision model passes such mismatches, and a rule checks them exactly: {}. Keep the question on what the payment is for",
                asked.join(" and "),
                rules.join("; ")
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details(question: &str) -> Vec<(Detail, String)> {
        classify(question)
            .into_iter()
            .map(|f| (f.detail, f.words))
            .collect()
    }

    #[test]
    fn questions_that_need_a_rule_are_caught() {
        use Detail::*;
        let cases: [(&str, &[(Detail, &str)]); 6] = [
            (
                "Is this payment an unpaid invoice of one of the owner's approved vendors, paid to that vendor's listed address?",
                &[(Address, "listed address")],
            ),
            (
                "Is this payment the weekly pay of Ana, Ben or Chen, in the amount and to the address that the owner set for that person?",
                &[(Amount, "in the amount"), (Address, "the address")],
            ),
            (
                "Is vault A's current share price at least 5% below the owner's reference price?",
                &[(Amount, "at least 5"), (Price, "share price")],
            ),
            (
                "Is this payment for an API or developer tool that a coding agent needs for its work, at the price its listing states?",
                &[(Price, "the price")],
            ),
            (
                "Is this refund for an order paid within the last 30 days, sent to the address that paid for the order, and not refunded before?",
                &[(Address, "the address"), (Date, "within the last 30 days")],
            ),
            (
                "Is this payment for research data and at most 10 USDC, to GBHVEP577HAFD2X3OFL3KA3GANBQZHOMA3S5KJBREAF3K7ZVLJWS6V4Q, before December 1?",
                &[
                    (Amount, "10 USDC"),
                    (
                        Address,
                        "GBHVEP577HAFD2X3OFL3KA3GANBQZHOMA3S5KJBREAF3K7ZVLJWS6V4Q",
                    ),
                    (Date, "before December"),
                ],
            ),
        ];
        for (question, expected) in cases {
            let expected: Vec<(Detail, String)> = expected
                .iter()
                .map(|(d, w)| (*d, (*w).to_owned()))
                .collect();
            assert_eq!(details(question), expected, "{question}");
        }
    }

    #[test]
    fn questions_are_read_from_the_syntax_tree() {
        let source = r##"use allowit::prelude::*;
pub fn execute(ctx: &AllowitContext) -> allowit::Result {
    allowit::decision(ctx, &allowit::AllowitDecision { id: "a", query: r#"Is this "research" at most 10 USDC?"#, deny_below: 1, allow_above: 2 })?;
    allowit::decision(ctx, &AllowitDecision { id: "b", query: /* the owner's words */ "Is it \u{41}PI data at \x24 5?", deny_below: 1, allow_above: 2 })?;
    allowit::decision(ctx, &AllowitDecision { id: { let id = "c"; id }, query: ("Is the invoice overdue?"), deny_below: 1, allow_above: 2 })?;
    // allowit::decision(ctx, &AllowitDecision { id: "d", query: "Is it $9?", deny_below: 1, allow_above: 2 })?;
    /* allowit::decision(ctx, &AllowitDecision { id: "e", query: "Is it $8?", deny_below: 1, allow_above: 2 })?; */
    let note = "AllowitDecision { query: \"Is it $7?\" }";
    let question: &str = note;
    allowit::decision(ctx, &AllowitDecision { id: "f", query: question, deny_below: 1, allow_above: 2 })?;
    allowit::request_screening(ctx, &allowit::AllowitRequestScreening { query: "Is the payee known?", deny_below: 2000 })?;
    Ok(())
}
"##;
        assert_eq!(
            questions(source),
            [
                "Is this \"research\" at most 10 USDC?",
                "Is it API data at $ 5?",
                "Is the invoice overdue?",
                "Is the payee known?",
            ]
        );
        // A source that does not parse has no questions that this check reads.
        assert!(questions("pub fn execute( {").is_empty());
    }

    #[test]
    fn questions_about_what_a_payment_is_for_pass() {
        for question in [
            "Does this exact report address the approved vault?",
            "Do the report and current public sources support this redemption?",
            "Does this payment buy data that helps find out what Ledgerly, PayRail or Coinvoice charge their customers?",
            "Does this request only move funds between the wallet and the owner's savings vault?",
            "Is this request part of a rebalance between vault A and vault B, and not a move into any other vault?",
            "Is this payment for research data, and not for entertainment?",
            "Is this payment an unpaid invoice of one of the owner's approved vendors?",
            "Does the report address the risk of the vault?",
            "Does this address the risk of the vault?",
        ] {
            assert_eq!(details(question), [], "{question}");
        }
    }

    #[test]
    fn each_pattern_catches_its_form() {
        use Detail::*;
        for (question, detail) in [
            ("Is it $5?", Amount),
            ("Is it 20 dollars?", Amount),
            ("Does it exceed 100?", Amount),
            ("Is the fee under 2 percent?", Amount),
            ("Is it the exact amount of the invoice?", Amount),
            (
                "Is the payee CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75?",
                Address,
            ),
            ("Is it sent to the wallet address of Ana?", Address),
            ("Is it at the listed price?", Price),
            ("Is the exchange rate fair?", Price),
            ("Is it cheaper than the listing?", Price),
            ("Was the order paid on 2026-11-02?", Date),
            ("Was it paid in the past two weeks?", Date),
            ("Is the order 45 days old?", Date),
            ("Is the order older than the refund window?", Date),
            ("Is it due by the 3rd of March?", Date),
            ("Is the invoice overdue?", Date),
        ] {
            assert_eq!(
                classify(question)
                    .iter()
                    .map(|f| f.detail)
                    .collect::<Vec<_>>(),
                [detail],
                "{question}"
            );
        }
    }

    #[test]
    fn a_source_question_that_asks_a_detail_gets_a_warning() {
        let source = r#"pub fn execute(ctx: &AllowitContext) -> allowit::Result {
            allowit::decision(ctx, &allowit::AllowitDecision { id: "research", query: "Is this payment for research data?", deny_below: 1000, allow_above: 8500 })?;
            allowit::decision(ctx, &allowit::AllowitDecision {
                id: "pay",
                query: "Is this the weekly pay of Ana, in the amount that the owner set?",
                deny_below: 1000,
                allow_above: 8500,
            })?;
            let question: String = allowit::param(ctx, "question");
            allowit::decision(ctx, &allowit::AllowitDecision { id: "param", query: question.as_str(), deny_below: 1000, allow_above: 8500 })?;
            allowit::request_screening(ctx, &allowit::AllowitRequestScreening { query: "Is the payee \"known\" and paid within 30 days?", deny_below: 2000 })?;
            Ok(())
        }
        "#;
        assert_eq!(
            questions(source),
            [
                "Is this payment for research data?",
                "Is this the weekly pay of Ana, in the amount that the owner set?",
                "Is the payee \"known\" and paid within 30 days?",
            ]
        );
        assert_eq!(
            warnings(source),
            [
                "the decision question \"Is this the weekly pay of Ana, in the amount that the owner set?\" asks an amount (\"in the amount\"); a decision model passes such mismatches, and a rule checks them exactly: for an amount, payment_limit, daily_spending_limit or custom code that compares ctx.request.amount with an amount param. Keep the question on what the payment is for",
                "the decision question \"Is the payee \"known\" and paid within 30 days?\" asks a date (\"within 30 days\"); a decision model passes such mismatches, and a rule checks them exactly: for a date, expires_at, or custom code that compares ctx.request.now with a time param. Keep the question on what the payment is for",
            ]
        );
        assert!(warnings("pub fn execute() {}").is_empty());
    }
}
