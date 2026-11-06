//! CLI custody, preflight and exact receipt recovery over actual loopback Core RPC.
#[path = "../../engine/tests/support/owner_chain.rs"]
mod chain;
mod common;
#[path = "../../engine/tests/support/owner_server.rs"]
mod wire;
use common::{Sandbox, code, stderr};
use serde_json::Value;
fn prepare(server: &chain::RpcServer) -> Sandbox {
    let mut sandbox = Sandbox::new("owner-flow");
    sandbox.ok(&["rail", "use", "stellar:testnet"]);
    for (name, seed) in [("owner", 11), ("agent", 12)] {
        let file = sandbox.private_file(&format!("{name}.key"), &common::stellar_seed([seed; 32]));
        sandbox.ok(&["keys", "import", name, file.to_str().unwrap()]);
        std::fs::remove_file(file).unwrap();
    }
    sandbox.ok(&["keys", "use", "agent"]);
    sandbox.set("ALLOWIT_URL", "http://127.0.0.1:1");
    sandbox.ok(&["rail", "trust", "http://127.0.0.1:1", &chain::account(13)]);
    let _ = &server.url;
    sandbox
}
#[test]
fn cli_native_setup_and_keyless_status_preserve_agent_selection_and_original_signed_bytes() {
    let server = chain::RpcServer::new(chain::Chain::new(chain::selection().config));
    let sandbox = prepare(&server);
    let before = sandbox.json(&["config", "show"]);
    let out = sandbox.run(&[
        "--json",
        "--rpc-url",
        &server.url,
        "wallet",
        "setup",
        "--owner-key",
        "owner",
        "--agent-key",
        "agent",
        "--server",
        &chain::account(13),
        "--request-id",
        "native-cli-001",
        "--yes",
    ]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    let setup: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(setup["state"], "configured");
    assert_eq!(sandbox.json(&["config", "show"]), before);
    let signed = server.chain.direct_signed.lock().unwrap().clone().unwrap();
    std::fs::remove_file(sandbox.config().join("secrets/owner")).unwrap();
    let out = sandbox.run(&[
        "--json",
        "--rpc-url",
        &server.url,
        "policy",
        "owner-status",
        "native-cli-001",
    ]);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    let status: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(status["transactions"][0]["status"], "confirmed");
    assert_eq!(
        server.chain.direct_signed.lock().unwrap().as_ref(),
        Some(&signed)
    );
    assert_eq!(
        server
            .chain
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v.as_str() == "sendTransaction")
            .count(),
        1
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(!text.contains(&common::stellar_seed([11; 32])));
}
#[test]
fn owner_command_preflight_refuses_implicit_keys_conflicting_roles_and_missing_rpc_before_signing()
{
    let server = chain::RpcServer::new(chain::Chain::new(chain::selection().config));
    let sandbox = prepare(&server);
    for network in ["stellar:testnet", "stellar:pubnet"] {
        let shown = sandbox.json(&["--rail", network, "rail", "show"]);
        let available = shown["available"].as_array().unwrap();
        for verb in [
            "wallet import",
            "policy deploy",
            "policy fund",
            "policy activate",
            "policy export",
            "policy owner-status",
        ] {
            assert!(
                available.contains(&serde_json::json!(verb)),
                "{network} missing {verb}"
            );
        }
        assert_eq!(
            available.contains(&serde_json::json!("wallet setup")),
            network == "stellar:testnet"
        );
    }
    let out = sandbox.run(&[
        "wallet",
        "setup",
        "--agent-key",
        "agent",
        "--server",
        &chain::account(13),
        "--request-id",
        "native-bad-001",
        "--yes",
    ]);
    assert_eq!(code(&out), Some(2));
    let out = sandbox.run(&[
        "wallet",
        "setup",
        "--owner-key",
        "owner",
        "--agent-key",
        "agent",
        "--server",
        &chain::account(13),
        "--request-id",
        "native-bad-001",
        "--yes",
    ]);
    assert_eq!(code(&out), Some(3));
    let out = sandbox.run(&[
        "--rpc-url",
        &server.url,
        "wallet",
        "setup",
        "--owner-key",
        "owner",
        "--agent-key",
        "agent",
        "--server",
        &chain::account(12),
        "--request-id",
        "native-bad-001",
        "--yes",
    ]);
    assert_eq!(code(&out), Some(2), "{}", stderr(&out));
    assert!(server.chain.events.lock().unwrap().is_empty());
}

#[test]
fn cli_import_verifies_portable_native_consent_then_resolves_without_a_bundle_file() {
    let selected = chain::native_selection();
    let ledger = chain::Chain::new(selected.config.clone());
    *ledger.source.lock().unwrap() = chain::configured_source(&selected.config);
    let rpc = chain::RpcServer::new(ledger);
    let mut sandbox = Sandbox::new("key-free-native-import");
    sandbox.ok(&["rail", "use", "stellar:testnet"]);
    sandbox.set("ALLOWIT_URL", "http://127.0.0.1:1");
    let input = sandbox.work().join("pre-trust.json");
    std::fs::write(
        &input,
        serde_json::to_vec(&chain::approved_descriptor(&selected)).unwrap(),
    )
    .unwrap();
    let missing = sandbox.run(&[
        "--rpc-url",
        &rpc.url,
        "wallet",
        "import",
        input.to_str().unwrap(),
    ]);
    assert_eq!(code(&missing), Some(3));
    assert!(stderr(&missing).contains("independent server authority"));
    assert!(!sandbox.config().join("public-profiles").exists());
    sandbox.ok(&["rail", "trust", "http://127.0.0.1:1", &chain::account(13)]);
    let configured = sandbox.json(&["config", "show"]);
    sandbox.ok(&["rail", "trust", "http://127.0.0.1:1", &chain::account(13)]);
    assert_eq!(sandbox.json(&["config", "show"]), configured);
    let refused = sandbox.run(&[
        "rail",
        "trust",
        "http://user:private-origin-sentinel@localhost",
        "private-key-sentinel",
    ]);
    assert_ne!(code(&refused), Some(0));
    assert!(!stderr(&refused).contains("private-origin-sentinel"));
    assert!(!stderr(&refused).contains("private-key-sentinel"));
    let before = sandbox.json(&["config", "show"]);
    let input = sandbox.work().join("public-profile.json");
    std::fs::write(
        &input,
        serde_json::to_vec(&chain::descriptor(&selected)).unwrap(),
    )
    .unwrap();
    let args = [
        "--json",
        "--rpc-url",
        &rpc.url,
        "wallet",
        "import",
        input.to_str().unwrap(),
    ];
    let refused = sandbox.run(&args);
    assert_eq!(code(&refused), Some(3));
    assert!(stderr(&refused).contains("administrator source approval"));
    std::fs::write(
        &input,
        serde_json::to_vec(&chain::approved_descriptor(&selected)).unwrap(),
    )
    .unwrap();
    let imported = sandbox.run(&args);
    assert_eq!(code(&imported), Some(0), "{}", stderr(&imported));
    let profile: Value = serde_json::from_slice(&imported.stdout).unwrap();
    assert_eq!(profile["wallet"], selected.config.wallet);
    assert!(profile["version"].is_string());
    assert_eq!(sandbox.json(&["config", "show"]), before);
    std::fs::remove_file(input).unwrap();
    assert!(!sandbox.config().join("keys").exists());
    assert!(!sandbox.config().join("secrets").exists());
    assert_eq!(
        sandbox.json(&["wallet", "show"])["version"],
        profile["version"]
    );
    assert_eq!(
        sandbox.ok(&["policy", "show", "--source"]).trim_end(),
        selected.policy.code.trim_end()
    );
}
