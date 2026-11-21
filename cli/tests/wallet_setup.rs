//! `wallet setup`: the owner confirms the network and, with `--amount`, the amount and the asset
//! of the new wallet.
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

/// Without `--yes` and without a terminal, the setup names its network and asks for `--yes`
/// before it reaches the server: the read of the wallet's asset signs the owner in, so it runs
/// only for a question that the owner sees.
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
        mock.assert_untouched();
    });
}

/// A setup is a resume only when the rail's journal holds its ID. A first attempt that stopped
/// at an unreachable RPC endpoint left nothing in the journal, so its retry is a new setup: the
/// question names its network and the setup that it creates, and no server is reached.
#[test]
#[cfg_attr(not(feature = "e2e"), ignore = "end-to-end: run with --features e2e")]
fn a_retry_of_a_setup_that_left_no_journal_is_a_new_setup() {
    each_rail(Feature::OwnerSetup, |rail| {
        let server = server_key(rail, 1);
        let mut sandbox = Sandbox::new("setup-retry");
        let mock = Mock::new();
        sandbox.set("ALLOWIT_URL", &mock.url);
        sandbox.ok(&["--rail", rail.network, "rail", "use", rail.network]);
        sandbox.ok(&["--rail", rail.network, "rail", "trust", &mock.url, &server]);
        sandbox.ok(&["keys", "generate", "owner"]);
        sandbox.ok(&["keys", "generate", "agent"]);
        let setup = |yes: bool| {
            let mut args = vec![
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
                "setup-0002",
                "--amount",
                "20",
            ];
            if yes {
                args.insert(0, "--yes");
            }
            sandbox.run(&args)
        };
        // The first run starts the setup and stops at the unreachable RPC endpoint.
        let first = setup(true);
        assert_ne!(code(&first), Some(0), "{}", stderr(&first));
        // The retry asks again, for a new setup.
        let retry = setup(false);
        let text = stderr(&retry);
        assert_eq!(code(&retry), Some(2), "{text}");
        assert!(text.contains("pass --yes"), "{text}");
        assert!(!text.contains("Resume"), "{text}");
        assert!(
            text.contains(&format!(
                "Create this agent's wallet account on {} from",
                rail.network
            )),
            "{text}"
        );
        mock.assert_untouched();
    });
}
