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
    // A Simple or Standard wallet stores no owner text, and its rail checks the source with
    // the portable profile: the same policy is refused for it, as its deployment would be.
    for kind in ["simple", "standard"] {
        refused(
            &["policy", "check", "asks.rs", "--kind", kind],
            &["the method `as_str`", "--kind native", "--kind custom"],
        );
    }
    // A Custom wallet takes the same one source. The server evaluates all of it, and the
    // wallet contract compiles its on-chain part: the limit. The question and the owner's
    // text that only the question reads leave.
    for args in [
        &["policy", "check", "asks.rs", "--kind", "custom"][..],
        &[
            "policy",
            "check",
            "asks.rs",
            "--kind",
            "custom",
            "--compile",
        ],
    ] {
        let checked = sandbox.json(args);
        assert_eq!(checked["profile"], "server", "{args:?}");
        assert_eq!(checked["placement"], "contract and server", "{args:?}");
        assert_eq!(checked["serverFunctions"], called, "{args:?}");
        assert_eq!(checked["sourceHash"], allowit_sdk::digest(asks.as_bytes()));
        assert_eq!(
            checked["onchain"]["kept"],
            serde_json::json!(["payment_limit"])
        );
        assert_eq!(checked["onchain"]["serverOnly"], serde_json::json!([]));
        assert_eq!(checked["onchain"]["left"], serde_json::json!([[4, 11]]));
        // A payment limit reads no cumulated figure: the wallet keeps none.
        assert_eq!(checked["onchain"]["figures"], serde_json::json!([]));
        assert_eq!(checked["compiled"], args.contains(&"--compile"));
    }
    // `--onchain` prints the part itself, byte for byte: the source without the lines that
    // left. Its hash is the one that the result states.
    let part = format!("{HEAD}    Ok(())\n}}\n");
    for args in [
        &[
            "policy",
            "check",
            "asks.rs",
            "--kind",
            "custom",
            "--onchain",
        ][..],
        &["policy", "check", "asks.rs", "--onchain"],
    ] {
        let output = sandbox.run(args);
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        assert_eq!(String::from_utf8_lossy(&output.stdout), part, "{args:?}");
    }
    // With `--json`, stdout stays one JSON document: the part is its `source` field.
    let printed = sandbox.json(&["policy", "check", "asks.rs", "--onchain"]);
    assert_eq!(printed, serde_json::json!({"source": part}));
    let checked = sandbox.json(&["policy", "check", "asks.rs", "--kind", "custom"]);
    assert_eq!(
        checked["onchain"]["sourceHash"],
        allowit_sdk::digest(part.as_bytes())
    );
    refused(
        &[
            "policy",
            "check",
            "asks.rs",
            "--kind",
            "native",
            "--onchain",
        ],
        &["--kind custom"],
    );
    // No other kind has an on-chain part of this form.
    let native = sandbox.json(&["policy", "check", "asks.rs", "--kind", "native"]);
    assert!(native["onchain"].is_null());

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
    let custom = sandbox.json(&["policy", "check", "states.rs", "--kind", "custom"]);
    assert_eq!(custom["placement"], "contract and server");
    assert_eq!(
        custom["onchain"]["kept"],
        serde_json::json!(["payment_limit"])
    );

    // Owner text without a server function: only a keypair wallet's policy.
    let checked = sandbox.json(&["policy", "check", "reads.rs"]);
    assert_eq!(
        (&checked["profile"], &checked["placement"]),
        (&serde_json::json!("server"), &serde_json::json!("server"))
    );
    assert_eq!(checked["serverFunctions"], serde_json::json!([]));
    for kind in ["simple", "standard"] {
        refused(
            &["policy", "check", "reads.rs", "--kind", kind],
            &["the method `as_str`", "--kind native"],
        );
    }
    // Only the server can read owner text. In a Custom wallet the statement that reads it
    // leaves the on-chain part, and the server evaluates it.
    let custom = sandbox.json(&["policy", "check", "reads.rs", "--kind", "custom"]);
    assert_eq!(custom["placement"], "contract and server");
    assert_eq!(
        custom["onchain"]["kept"],
        serde_json::json!(["payment_limit"])
    );
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
    // A Custom wallet's source without a server function is its own on-chain part: the
    // contract runs all of it.
    let whole = sandbox.json(&["policy", "check", "policy.rs", "--kind", "custom"]);
    assert_eq!(whole["placement"], "contract");
    assert_eq!(whole["onchain"]["sourceHash"], whole["sourceHash"]);
    assert_eq!(whole["onchain"]["left"], serde_json::json!([]));
    // The daily spending limit reads today's spending: the wallet keeps those daily figures.
    assert_eq!(
        whole["onchain"]["figures"],
        serde_json::json!([{"key": "day_spent", "daily": true}, {"key": "day_unpriced", "daily": true}])
    );
    let output = sandbox.run(&["policy", "check", "policy.rs", "--onchain"]);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        include_str!("../templates/policy/src/policy.rs")
    );

    // A rule in a branch whose condition only the server can evaluate runs only on the server: the
    // result names it, and a warning tells the owner to move it. A policy whose on-chain
    // part keeps no canonical rule, assertion or `return Err(...)` is admitted, and a warning
    // says so.
    let branched = states.replace(
        "    })?;\n    Ok(())",
        "    })?;\n    if ctx.request.amount > 5 {\n        allowit::destinations(ctx, &ctx.params[allowit::DESTINATIONS])?;\n    }\n    Ok(())",
    );
    let branched = branched.replace(
        "    allowit::decision(ctx,",
        "    let score = allowit::decision(ctx,",
    );
    let branched = branched.replace("if ctx.request.amount > 5 {", "if score < 9000 {");
    assert!(branched.contains("let score = ") && branched.contains("if score < 9000"));
    std::fs::write(sandbox.work().join("branched.rs"), &branched).unwrap();
    let output = sandbox.run(&[
        "--json",
        "policy",
        "check",
        "branched.rs",
        "--kind",
        "custom",
    ]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    let checked: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        checked["onchain"]["serverOnly"],
        serde_json::json!([{"line": 12, "check": "destinations", "cause": "server_branch", "movable": true}])
    );
    for word in [
        "Line 12",
        "destinations runs only on the server",
        "a branch whose condition only the server can evaluate",
        "Move it out",
    ] {
        assert!(stderr(&output).contains(word), "{}", stderr(&output));
    }
    let only = states.replace(
        "    allowit::payment_limit(ctx, &ctx.params[allowit::PAYMENT_LIMIT])?;\n",
        "",
    );
    std::fs::write(sandbox.work().join("only.rs"), &only).unwrap();
    let output = sandbox.run(&["--json", "policy", "check", "only.rs", "--kind", "custom"]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    assert!(
        stderr(&output)
            .contains("No canonical rule, assertion or `return Err(...)` of this policy stays"),
        "{}",
        stderr(&output)
    );
    // A policy for a Custom wallet may keep its own state, and the check names the keeper of
    // each key. The wallet contract keeps the count, whose statements depend on no server
    // function. The server keeps the score, and the check that reads it runs only there.
    let written = branched.replace(
        "    if score < 9000 {",
        "    let last: Option<u32> = allowit::get_state(ctx, \"last_score\")?;\n    assert!(last != Some(0));\n    allowit::set_state(ctx, \"last_score\", score)?;\n    if score < 9000 {",
    ).replace(
        "    let score = allowit::decision(ctx,",
        "    let paid: u64 = allowit::get_state(ctx, \"day_paid\")?.unwrap_or(0);\n    allowit::set_state(ctx, \"day_paid\", paid + 1)?;\n    let score = allowit::decision(ctx,",
    );
    assert!(written.contains("day_paid") && written.contains("last_score"));
    std::fs::write(sandbox.work().join("written.rs"), &written).unwrap();
    let output = sandbox.run(&[
        "--json",
        "policy",
        "check",
        "written.rs",
        "--kind",
        "custom",
    ]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    let checked: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        checked["onchain"]["state"],
        serde_json::json!([
            {"key": "day_paid", "keeper": "chain", "daily": true},
            {"key": "last_score", "keeper": "server", "daily": false}
        ])
    );
    let stored_state: Vec<&serde_json::Value> = checked["onchain"]["serverOnly"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|check| check["cause"] == "stored_state")
        .collect();
    assert_eq!(stored_state.len(), 1);
    assert_eq!(stored_state[0]["movable"], false);
    let output = sandbox.run(&[
        "policy",
        "check",
        "written.rs",
        "--kind",
        "custom",
        "--onchain",
    ]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    let part = String::from_utf8(output.stdout).unwrap();
    assert!(
        part.contains("\"day_paid\"") && !part.contains("last_score"),
        "{part}"
    );
    // Only a Custom wallet keeps a policy's own state.
    for kind in ["native", "standard"] {
        refused(
            &["policy", "check", "written.rs", "--kind", kind],
            &["stores no policy state", "Choose the Custom wallet"],
        );
    }
    // A source with no server function that keeps state is its own on-chain part.
    let plain_write = include_str!("../templates/policy/src/policy.rs").replace(
        "    Ok(())\n}",
        "    allowit::set_state(ctx, \"last\", ctx.request.amount)?;\n    Ok(())\n}",
    );
    assert!(plain_write.contains("set_state"));
    std::fs::write(sandbox.work().join("plain-write.rs"), &plain_write).unwrap();
    let output = sandbox.run(&[
        "--json",
        "policy",
        "check",
        "plain-write.rs",
        "--kind",
        "custom",
    ]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    let checked: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        checked["onchain"]["state"],
        serde_json::json!([{"key": "last", "keeper": "chain", "daily": false}])
    );
    assert_eq!(checked["placement"], "contract");
    // The contract must know each key before it runs.
    let dynamic = plain_write.replace(
        "allowit::set_state(ctx, \"last\", ctx.request.amount)?;",
        "let key = \"last\";\n    allowit::set_state(ctx, key, ctx.request.amount)?;",
    );
    std::fs::write(sandbox.work().join("dynamic.rs"), &dynamic).unwrap();
    refused(
        &["policy", "check", "dynamic.rs", "--kind", "custom"],
        &["string literal"],
    );
    // A read of a standard figure stays, and the wallet keeps that figure.
    let stored = branched.replace(
        "    let score = allowit::decision(ctx,",
        "    let calls: Option<u64> = allowit::get_state(ctx, \"calls\")?;\n    assert!(calls != Some(1_000));\n    let score = allowit::decision(ctx,",
    );
    std::fs::write(sandbox.work().join("stored.rs"), &stored).unwrap();
    let output = sandbox.run(&["--json", "policy", "check", "stored.rs", "--kind", "custom"]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    let checked: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        checked["onchain"]["figures"],
        serde_json::json!([{"key": "calls", "daily": false}])
    );
    assert!(
        checked["onchain"]["kept"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("assert"))
    );
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
    // Within the limit, Core's derivation reads the tokens: a comment inside the path does
    // not keep the server function in the on-chain part.
    std::fs::write(
        sandbox.work().join("spaced-small.rs"),
        states.replace("allowit::decision(", "allowit::/* note */decision("),
    )
    .unwrap();
    let spaced = sandbox.json(&["policy", "check", "spaced-small.rs", "--kind", "custom"]);
    assert_eq!(
        spaced["onchain"]["kept"],
        serde_json::json!(["payment_limit"])
    );
    let output = sandbox.run(&["policy", "check", "spaced-small.rs", "--onchain"]);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("decision"));
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
    // A raw string literal is a literal too.
    std::fs::write(
        sandbox.work().join("raw.rs"),
        source("Q").replace(
            "\"Q\"",
            "r#\"Is this payment for \"research\" data and at most 10 USDC?\"#",
        ),
    )
    .unwrap();
    let output = sandbox.run(&["policy", "check", "raw.rs"]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    assert!(
        stderr(&output).starts_with("warning: the decision question \"Is this payment for \"research\" data and at most 10 USDC?\" asks an amount (\"10 USDC\")"),
        "{}",
        stderr(&output)
    );
    // A question about what the payment is for gets none.
    let output = sandbox.run(&["policy", "check", "plain.rs"]);
    assert_eq!(code(&output), Some(0));
    assert_eq!(stderr(&output), "");
}
