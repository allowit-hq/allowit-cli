//! `wallet setup`: the owner confirms the network and, with `--amount`, the amount of the new
//! wallet.
mod common;

use allowit_sdk::plugins::Feature;
use common::{Mock, Rail, Sandbox, code, each_rail, stderr};

fn server_key(rail: &Rail<'_>, seed: u8) -> String {
    let keys = rail.plugin.keys();
    keys.derive(&vec![seed; keys.entropy_bytes()])
        .unwrap()
        .address()
        .to_owned()
}

/// Without `--yes` and without a terminal, the setup names its network and its amount and asks
/// for `--yes` before it reaches the server: nothing signs the owner in before the answer.
#[test]
#[cfg_attr(not(feature = "e2e"), ignore = "end-to-end: run with --features e2e")]
fn a_setup_without_yes_names_its_network_and_reaches_no_server() {
    each_rail(Feature::OwnerSetup, |rail| {
        let server = server_key(rail, 1);
        let mut sandbox = Sandbox::new("setup-confirm");
        let mock = Mock::new();
        sandbox.set("ALLOWIT_URL", &mock.url);
        sandbox.ok(&["--rail", rail.network, "rail", "use", rail.network]);
        sandbox.ok(&["--rail", rail.network, "rail", "trust", &mock.url, &server]);
        sandbox.ok(&["keys", "generate", "owner"]);
        sandbox.ok(&["keys", "generate", "agent"]);
        let output = sandbox.run(&[
            "--rpc-url",
            "http://127.0.0.1:1",
            "wallet",
            "setup",
            "--owner-key",
            "owner",
            "--origin",
            "http://localhost:5173",
            "--agent-key",
            "agent",
            "--server",
            &server,
            "--request-id",
            "setup-0001",
            "--amount",
            "20",
        ]);
        assert_eq!(code(&output), Some(2), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("pass --yes"),
            "{}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(&format!("on {} from", rail.network)),
            "{}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("with 20 of the wallet's asset for the agent's payments"),
            "{}",
            stderr(&output)
        );
        mock.assert_untouched();
    });
}
