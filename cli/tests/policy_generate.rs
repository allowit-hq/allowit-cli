//! CLI key binding over the shared owner workflow. Model responses are recorded protocol replay.
mod common;
#[path = "../../engine/tests/support/owner_server.rs"]
mod wire;
use common::{Sandbox, code, stderr};
use serde_json::Value;

fn prepare(server: &wire::Server) -> Sandbox {
    let mut sandbox = Sandbox::new("owner-generate");
    sandbox.ok(&["rail", "use", "stellar:testnet"]);
    for (name, seed) in [("owner", 11), ("agent", 12)] {
        let path = sandbox.private_file(&format!("{name}.key"), &common::stellar_seed([seed; 32]));
        sandbox.ok(&["keys", "import", name, path.to_str().unwrap()]);
        std::fs::remove_file(path).unwrap();
    }
    sandbox.ok(&["keys", "use", "agent"]);
    sandbox.set("ALLOWIT_URL", &server.url);
    sandbox
}
fn args(prompt: &str) -> Vec<&str> {
    vec![
        "--json",
        "policy",
        "generate",
        "--owner-key",
        "owner",
        "--origin",
        "http://localhost:5173",
        "--profile",
        "allowit-stellar-policy-wallet-v1",
        "--new-wallet",
        prompt,
    ]
}
#[test]
fn owner_generation_calls_the_server_and_keeps_agent_selection_and_keys_local() {
    let owner = allowit_stellar_wallet::account_for_seed(&[11; 32]);
    let server = wire::Server::new(wire::Plan::new(&owner));
    let sandbox = prepare(&server);
    let before = sandbox.json(&["config", "show"]);
    let prompt = wire::fixture()["prompt"].as_str().unwrap().to_owned();
    let out = sandbox.run(&args(&prompt));
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    let policy: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(policy["originalIntent"], prompt);
    assert_eq!(policy["status"], "draft");
    assert!(
        policy["runtime"]["instanceBinding"]["template"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(server.calls("/api/micro/generate"), 1);
    assert_eq!(sandbox.json(&["config", "show"]), before);
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), stderr(&out));
    assert!(!text.contains(&common::stellar_seed([11; 32])));
    assert!(!text.contains(&"s".repeat(32)));
}
#[test]
fn no_implicit_owner_key_or_conflicting_selection_can_reach_authentication() {
    let owner = allowit_stellar_wallet::account_for_seed(&[11; 32]);
    let server = wire::Server::new(wire::Plan::new(&owner));
    let sandbox = prepare(&server);
    let prompt = "Pay only the vendor";
    let mut missing = args(prompt);
    let at = missing.iter().position(|v| *v == "--owner-key").unwrap();
    missing.drain(at..at + 2);
    assert_eq!(code(&sandbox.run(&missing)), Some(2));
    let mut wrong = args(prompt);
    let at = wrong.iter().position(|v| *v == "owner").unwrap();
    wrong[at] = "missing-owner";
    assert_eq!(code(&sandbox.run(&wrong)), Some(3));
    let mut conflicting = args(prompt);
    conflicting.extend([
        "--installation",
        "0707070707070707070707070707070707070707070707070707070707070707",
    ]);
    assert_eq!(code(&sandbox.run(&conflicting)), Some(2));
    assert_eq!(server.calls("/api/auth/challenge"), 0);
    assert_eq!(server.calls("/api/micro/generate"), 0);
}
#[test]
fn unresolved_generation_is_owner_input_and_transport_loss_is_uncertain_without_retry() {
    let owner = allowit_stellar_wallet::account_for_seed(&[11; 32]);
    for lost in [false, true] {
        let mut plan = wire::Plan::new(&owner);
        plan.lost_generation = lost;
        if !lost {
            plan.policy["runtime"]["instanceBinding"]["portable"]["unresolved"] =
                serde_json::json!(["destinations"]);
        }
        let server = wire::Server::new(plan);
        let sandbox = prepare(&server);
        let prompt = wire::fixture()["prompt"].as_str().unwrap().to_owned();
        let out = sandbox.run(&args(&prompt));
        assert_eq!(
            code(&out),
            Some(if lost { 5 } else { 11 }),
            "{}",
            stderr(&out)
        );
        assert_eq!(server.calls("/api/micro/generate"), 1);
        if lost {
            assert!(out.stdout.is_empty());
        } else {
            let value: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(value["status"], "draft");
        }
    }
}
