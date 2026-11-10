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
