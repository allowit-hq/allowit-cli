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

/// A policy is checked with the profile of the place that evaluates it. The paper's kind of
/// policy reads its owner's text from params and passes it to server functions: the server
/// evaluates it, and the check gives the verdict of the server's gate. A contract compiles
/// every other policy, and the portable profile checks it.
#[test]
fn check_uses_the_profile_of_the_place_that_evaluates_the_policy() {
    const HEAD: &str = "use allowit::prelude::*;\npub fn execute(ctx: &AllowitContext) -> allowit::Result {\n    allowit::payment_limit(ctx, &ctx.params[allowit::PAYMENT_LIMIT])?;\n";
    let asks = format!(
        "{HEAD}    let question: String = allowit::param(ctx, \"question\");\n    allowit::new_decision_context(ctx, None)?;\n    allowit::decision(ctx, &allowit::AllowitDecision {{\n        id: \"purchase\",\n        query: question.as_str(),\n        deny_below: 3000,\n        allow_above: 8000,\n    }})?;\n    Ok(())\n}}\n"
    );
    let reads = format!(
        "{HEAD}    let note: String = allowit::param(ctx, \"note\");\n    match note.as_str() {{\n        \"open\" => Ok(()),\n        _ => Err(AllowitError::InvalidParam),\n    }}\n}}\n"
    );
    let sandbox = Sandbox::new("placement");
    std::fs::write(sandbox.work().join("asks.rs"), &asks).unwrap();
    std::fs::write(sandbox.work().join("reads.rs"), &reads).unwrap();
    std::fs::write(
        sandbox.work().join("policy.rs"),
        include_str!("../templates/policy/src/policy.rs"),
    )
    .unwrap();

    // A policy that calls a server function: the server profile, also with --compile.
    let checked = sandbox.json(&["policy", "check", "asks.rs"]);
    assert_eq!(checked["placement"], "server");
    assert_eq!(
        checked["serverFunctions"],
        serde_json::json!(["allowit::new_decision_context", "allowit::decision"])
    );
    assert_eq!(checked["prefix"], serde_json::json!(["payment_limit"]));
    assert_eq!(checked["sourceHash"], allowit_sdk::digest(asks.as_bytes()));
    for kind in ["native", "simple", "standard"] {
        let checked = sandbox.json(&["policy", "check", "asks.rs", "--kind", kind]);
        assert_eq!(checked["placement"], "server", "{kind}");
    }
    let compiled = sandbox.json(&["policy", "check", "asks.rs", "--compile"]);
    assert_eq!(
        (&compiled["compiled"], &compiled["placement"]),
        (&serde_json::json!(true), &serde_json::json!("server"))
    );
    // A Custom wallet compiles its whole source on chain: it cannot hold that policy.
    let refused = sandbox.run(&["policy", "check", "asks.rs", "--kind", "custom"]);
    assert_eq!(code(&refused), Some(2), "{}", stderr(&refused));
    assert!(
        stderr(&refused).contains("allowit::decision"),
        "{}",
        stderr(&refused)
    );
    assert!(refused.stdout.is_empty());

    // Owner text without a server function: only a keypair wallet's policy may read it,
    // because only the server evaluates that policy.
    let refused = sandbox.run(&["policy", "check", "reads.rs"]);
    assert_eq!(code(&refused), Some(2), "{}", stderr(&refused));
    assert!(
        stderr(&refused).contains("the method `as_str`"),
        "{}",
        stderr(&refused)
    );
    assert!(
        stderr(&refused).contains("--kind native"),
        "{}",
        stderr(&refused)
    );
    for kind in ["simple", "standard", "custom"] {
        let refused = sandbox.run(&["policy", "check", "reads.rs", "--kind", kind]);
        assert_eq!(code(&refused), Some(2), "{kind}: {}", stderr(&refused));
    }
    let native = sandbox.json(&[
        "policy",
        "check",
        "reads.rs",
        "--kind",
        "native",
        "--compile",
    ]);
    assert_eq!(native["placement"], "server");
    assert_eq!(native["serverFunctions"], serde_json::json!([]));
    assert_eq!(native["compiled"], true);

    // Source for a contract: the portable profile, as before.
    let portable = sandbox.json(&["policy", "check", "policy.rs"]);
    assert_eq!(portable["placement"], "contract");
    assert_eq!(portable["serverFunctions"], serde_json::json!([]));
    // A type error fails the compilation of a server policy too.
    std::fs::write(
        sandbox.work().join("wrong.rs"),
        asks.replace("deny_below: 3000", "deny_below: true"),
    )
    .unwrap();
    sandbox.ok(&["policy", "check", "wrong.rs"]);
    let failed = sandbox.run(&["policy", "check", "wrong.rs", "--compile"]);
    assert_eq!(code(&failed), Some(2), "{}", stderr(&failed));
    assert!(stderr(&failed).contains("mismatched types"));
}
