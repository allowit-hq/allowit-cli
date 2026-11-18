//! Structural admission and real host compilation have different outcomes.
mod common;
use common::{Sandbox, code, stderr};

#[test]
fn check_compile_type_checks_what_structural_admission_accepts() {
    let mut sandbox = Sandbox::new("portable-build");
    let source = include_str!("../templates/policy/src/policy.rs");
    std::fs::write(sandbox.work().join("policy.rs"), source).unwrap();
    let checked = sandbox.json(&["policy", "check", "policy.rs"]);
    assert_eq!(
        checked["sourceHash"],
        allowit_sdk::digest(source.as_bytes())
    );
    assert!(checked.get("ir").is_none());
    assert_eq!(checked["compiled"], false);
    let compiled = sandbox.json(&["policy", "check", "policy.rs", "--compile"]);
    assert_eq!(compiled["compiled"], true);
    assert_eq!(compiled["sourceHash"], checked["sourceHash"]);

    let invalid = "use allowit::prelude::*; pub fn execute(ctx: &AllowitContext) -> allowit::Result { let value: i128 = true; let _ = value; let _ = ctx; Ok(()) }";
    std::fs::write(sandbox.work().join("invalid.rs"), invalid).unwrap();
    sandbox.ok(&["policy", "check", "invalid.rs"]);
    let failed = sandbox.run(&["policy", "check", "invalid.rs", "--compile"]);
    assert_eq!(code(&failed), Some(2), "{}", stderr(&failed));
    assert!(stderr(&failed).contains("mismatched types"));
    assert!(failed.stdout.is_empty());

    sandbox.set("PATH", "/allowit-no-host-tools");
    let failed = sandbox.run(&["policy", "check", "policy.rs", "--compile"]);
    assert_eq!(code(&failed), Some(3), "{}", stderr(&failed));
    assert!(failed.stdout.is_empty());
    // The structural check needs no host tools.
    sandbox.ok(&["policy", "check", "policy.rs"]);
}

/// A policy is checked with the profile of the wallet kind that holds it. The paper's kind of
/// policy reads its owner's text from params and passes it to server functions: only a keypair
/// wallet holds it, because the server keeps that wallet's params, and the check gives the
/// verdict of the server's gate. A contract wallet holds its source on chain: the portable
/// profile checks it, also when it calls a server function.
#[test]
fn check_uses_the_profile_of_the_wallet_kind_that_holds_the_policy() {
    const HEAD: &str = "use allowit::prelude::*;\npub fn execute(ctx: &AllowitContext) -> allowit::Result {\n    allowit::payment_limit(ctx, &ctx.params[allowit::PAYMENT_LIMIT])?;\n";
    // The question is the owner's text param.
    let asks = format!(
        "{HEAD}    let question: String = allowit::param(ctx, \"question\");\n    allowit::new_decision_context(ctx, None)?;\n    allowit::decision(ctx, &allowit::AllowitDecision {{\n        id: \"purchase\",\n        query: question.as_str(),\n        deny_below: 3000,\n        allow_above: 8000,\n    }})?;\n    Ok(())\n}}\n"
    );
    // The question is in the source.
    let states = asks
        .replace(
            "    let question: String = allowit::param(ctx, \"question\");\n",
            "",
        )
        .replace(
            "question.as_str()",
            "\"Is this payment for research data?\"",
        );
    // Owner text without a server function.
    let reads = format!(
        "{HEAD}    let note: String = allowit::param(ctx, \"note\");\n    match note.as_str() {{\n        \"open\" => Ok(()),\n        _ => Err(AllowitError::InvalidParam),\n    }}\n}}\n"
    );
    let sandbox = Sandbox::new("placement");
    for (name, source) in [
        ("asks.rs", &asks),
        ("states.rs", &states),
        ("reads.rs", &reads),
    ] {
        std::fs::write(sandbox.work().join(name), source).unwrap();
    }
    std::fs::write(
        sandbox.work().join("policy.rs"),
        include_str!("../templates/policy/src/policy.rs"),
    )
    .unwrap();
    let called = serde_json::json!(["allowit::new_decision_context", "allowit::decision"]);
    let refused = |args: &[&str], words: &[&str]| {
        let output = sandbox.run(args);
        assert_eq!(code(&output), Some(2), "{args:?}: {}", stderr(&output));
        assert!(output.stdout.is_empty(), "{args:?}");
        for word in words {
            assert!(
                stderr(&output).contains(word),
                "{args:?}: {}",
                stderr(&output)
            );
        }
    };

    // The paper's kind of policy: a keypair wallet's policy, checked with the server profile.
    for args in [
        &["policy", "check", "asks.rs"][..],
        &["policy", "check", "asks.rs", "--kind", "native"],
        &["policy", "check", "asks.rs", "--compile"],
    ] {
        let checked = sandbox.json(args);
        assert_eq!(checked["profile"], "server", "{args:?}");
        assert_eq!(checked["placement"], "server", "{args:?}");
        assert_eq!(checked["serverFunctions"], called, "{args:?}");
        assert_eq!(checked["prefix"], serde_json::json!(["payment_limit"]));
        assert_eq!(checked["sourceHash"], allowit_sdk::digest(asks.as_bytes()));
        assert_eq!(checked["compiled"], args.contains(&"--compile"));
    }
    // A contract wallet stores no owner text, and its rail checks the source with the
    // portable profile: the same policy is refused for it, as its deployment would be.
    for kind in ["simple", "standard"] {
        refused(
            &["policy", "check", "asks.rs", "--kind", kind],
            &["the method `as_str`", "--kind native"],
        );
    }
    refused(
        &["policy", "check", "asks.rs", "--kind", "custom"],
        &["allowit::decision"],
    );

    // A policy that calls a server function and states its text in the source: a Simple or
    // Standard wallet holds it, the portable profile checks it, and the server evaluates it.
    for args in [
        &["policy", "check", "states.rs"][..],
        &["policy", "check", "states.rs", "--kind", "simple"],
        &[
            "policy",
            "check",
            "states.rs",
            "--kind",
            "standard",
            "--compile",
        ],
    ] {
        let checked = sandbox.json(args);
        assert_eq!(checked["profile"], "portable", "{args:?}");
        assert_eq!(checked["placement"], "server", "{args:?}");
        assert_eq!(checked["serverFunctions"], called, "{args:?}");
    }
    let native = sandbox.json(&["policy", "check", "states.rs", "--kind", "native"]);
    assert_eq!(native["profile"], "server");
    refused(
        &["policy", "check", "states.rs", "--kind", "custom"],
        &["allowit::decision"],
    );

    // Owner text without a server function: only a keypair wallet's policy.
    let checked = sandbox.json(&["policy", "check", "reads.rs"]);
    assert_eq!(
        (&checked["profile"], &checked["placement"]),
        (&serde_json::json!("server"), &serde_json::json!("server"))
    );
    assert_eq!(checked["serverFunctions"], serde_json::json!([]));
    for kind in ["simple", "standard", "custom"] {
        refused(
            &["policy", "check", "reads.rs", "--kind", kind],
            &["the method `as_str`", "--kind native"],
        );
    }
    let native = sandbox.json(&[
        "policy",
        "check",
        "reads.rs",
        "--kind",
        "native",
        "--compile",
    ]);
    assert_eq!(native["compiled"], true);

    // Custom code without a server function: the Native wallet takes it, as its draft, the
    // adoption of its reviewed proposal and its approval do, because the server evaluates the
    // whole policy. A Simple or Standard wallet refuses it, as the server does.
    let custom = format!(
        "{HEAD}    if ctx.request.amount < 1 {{\n        return Err(AllowitError::PaymentLimitExceeded);\n    }}\n    Ok(())\n}}\n"
    );
    std::fs::write(sandbox.work().join("custom.rs"), &custom).unwrap();
    let native = sandbox.json(&["policy", "check", "custom.rs", "--kind", "native"]);
    assert_eq!(
        (
            &native["profile"],
            &native["placement"],
            &native["canonicalOnly"]
        ),
        (
            &serde_json::json!("server"),
            &serde_json::json!("server"),
            &serde_json::json!(false)
        )
    );
    assert_eq!(native["serverFunctions"], serde_json::json!([]));
    for kind in ["simple", "standard"] {
        refused(
            &["policy", "check", "custom.rs", "--kind", kind],
            &["other custom code", "Native wallet"],
        );
    }
    let contract = sandbox.json(&["policy", "check", "custom.rs", "--kind", "custom"]);
    assert_eq!(contract["placement"], "contract");

    // Source for a contract: the portable profile, as before.
    let portable = sandbox.json(&["policy", "check", "policy.rs"]);
    assert_eq!(portable["profile"], "portable");
    assert_eq!(portable["placement"], "contract");
    assert_eq!(portable["serverFunctions"], serde_json::json!([]));
    // The server's gate checks every policy with the server profile, which takes at most
    // 16000 bytes. A larger policy is refused for every kind, whether it names a server
    // function, also in a spelling that only a token reader sees, or none.
    let padding = format!("// {}\n", "padding ".repeat(2_100));
    for (name, source) in [
        ("large.rs", states.clone()),
        (
            "spaced.rs",
            states.replace("allowit::decision(", "allowit::/* note */decision("),
        ),
        (
            "raw.rs",
            states.replace("allowit::decision(", "allowit::r#decision("),
        ),
        (
            "plain.rs",
            include_str!("../templates/policy/src/policy.rs").to_owned(),
        ),
    ] {
        std::fs::write(sandbox.work().join(name), format!("{padding}{source}")).unwrap();
        for kind in [None, Some("custom"), Some("standard"), Some("native")] {
            let mut args = vec!["policy", "check", name];
            args.extend(kind.into_iter().flat_map(|kind| ["--kind", kind]));
            refused(&args, &["16000 bytes"]);
        }
    }
    // Within the limit, Core's gate reads the tokens: a comment inside the path does not
    // hide the server function from the Custom wallet's refusal.
    std::fs::write(
        sandbox.work().join("spaced-small.rs"),
        states.replace("allowit::decision(", "allowit::/* note */decision("),
    )
    .unwrap();
    refused(
        &["policy", "check", "spaced-small.rs", "--kind", "custom"],
        &["allowit::decision"],
    );
    // A type error fails the compilation of a server policy too.
    std::fs::write(
        sandbox.work().join("wrong.rs"),
        asks.replace("deny_below: 3000", "deny_below: true"),
    )
    .unwrap();
    sandbox.ok(&["policy", "check", "wrong.rs"]);
    refused(
        &["policy", "check", "wrong.rs", "--compile"],
        &["mismatched types"],
    );
}

/// A literal decision question that asks an amount, an address, a price or a date gets a
/// warning on stderr; the check still passes and its result keeps its shape. A question that a
/// policy reads from a param is not checked.
#[test]
fn check_warns_when_a_question_asks_what_a_rule_checks() {
    const HEAD: &str = "use allowit::prelude::*;\npub fn execute(ctx: &AllowitContext) -> allowit::Result {\n    allowit::payment_limit(ctx, &ctx.params[allowit::PAYMENT_LIMIT])?;\n";
    let source = |query: &str| {
        format!(
            "{HEAD}    allowit::decision(ctx, &allowit::AllowitDecision {{\n        id: \"purchase\",\n        query: \"{query}\",\n        deny_below: 1000,\n        allow_above: 8500,\n    }})?;\n    Ok(())\n}}\n"
        )
    };
    let sandbox = Sandbox::new("question-details");
    for (name, query) in [
        (
            "amount.rs",
            "Is this payment for research data and at most 10 USDC?",
        ),
        (
            "refund.rs",
            "Is this refund for an order paid within the last 30 days, sent to the address that paid for the order?",
        ),
        ("plain.rs", "Is this payment for research data?"),
    ] {
        std::fs::write(sandbox.work().join(name), source(query)).unwrap();
    }
    let output = sandbox.run(&["policy", "check", "amount.rs"]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "warning: the decision question \"Is this payment for research data and at most 10 USDC?\" asks an amount (\"10 USDC\"); a decision model passes such mismatches, and a rule checks them exactly: for an amount, payment_limit, daily_spending_limit or custom code that compares ctx.request.amount with an amount param. Keep the question on what the payment is for\n"
    );
    let output = sandbox.run(&["policy", "check", "refund.rs"]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    let warned = stderr(&output);
    assert!(
        warned
            .contains("asks an address (\"the address\") and a date (\"within the last 30 days\")"),
        "{warned}"
    );
    assert!(
        warned.contains("for an address, destinations with the addresses; for a date, expires_at"),
        "{warned}"
    );
    // JSON mode: the result is unchanged, and the warning is a JSON line on stderr.
    let checked = sandbox.json(&["policy", "check", "amount.rs"]);
    assert_eq!(
        checked["serverFunctions"],
        serde_json::json!(["allowit::decision"])
    );
    assert!(checked.get("warnings").is_none());
    let output = sandbox.run(&["--json", "policy", "check", "amount.rs"]);
    let line: serde_json::Value =
        serde_json::from_str(stderr(&output).lines().next().unwrap()).unwrap();
    assert!(
        line["warning"]["message"]
            .as_str()
            .unwrap()
            .contains("asks an amount")
    );
    // A question about what the payment is for gets none.
    let output = sandbox.run(&["policy", "check", "plain.rs"]);
    assert_eq!(code(&output), Some(0));
    assert_eq!(stderr(&output), "");
}
