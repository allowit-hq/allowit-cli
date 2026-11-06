//! Structural admission and real host compilation have different outcomes.
mod common;
use common::{Sandbox, code, stderr};

#[test]
fn portable_build_compiles_before_emitting_exact_source_and_preserves_failed_outputs() {
    let mut sandbox = Sandbox::new("portable-build");
    let source = include_str!("../templates/policy/src/policy.rs");
    std::fs::write(sandbox.work().join("policy.rs"), source).unwrap();
    let checked = sandbox.json(&["policy", "check", "policy.rs"]);
    assert_eq!(
        checked["sourceHash"],
        allowit_sdk::digest(source.as_bytes())
    );
    assert!(checked.get("ir").is_none());
    sandbox.ok(&["policy", "build", "policy.rs", "-o", "wallet.rs"]);
    assert_eq!(
        std::fs::read(sandbox.work().join("wallet.rs")).unwrap(),
        source.as_bytes()
    );

    let invalid = "use allowit::prelude::*; pub fn execute(ctx: &AllowitContext) -> allowit::Result { let value: i128 = true; let _ = value; let _ = ctx; Ok(()) }";
    std::fs::write(sandbox.work().join("invalid.rs"), invalid).unwrap();
    sandbox.ok(&["policy", "check", "invalid.rs"]);
    let failed = sandbox.run(&["policy", "build", "invalid.rs", "-o", "wallet.rs"]);
    assert_eq!(code(&failed), Some(2), "{}", stderr(&failed));
    assert!(stderr(&failed).contains("mismatched types"));
    assert!(failed.stdout.is_empty());
    assert_eq!(
        std::fs::read(sandbox.work().join("wallet.rs")).unwrap(),
        source.as_bytes()
    );

    sandbox.set("PATH", "/allowit-no-host-tools");
    let failed = sandbox.run(&["policy", "build", "policy.rs", "-o", "wallet.rs"]);
    assert_eq!(code(&failed), Some(3), "{}", stderr(&failed));
    assert_eq!(
        std::fs::read(sandbox.work().join("wallet.rs")).unwrap(),
        source.as_bytes()
    );
}
