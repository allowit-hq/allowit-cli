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

/// A setup ID that this CLI started before is a resume: the question says so and names no asset,
/// because the transaction that was prepared then keeps its own asset. The CLI reads no template
/// for it, so the owner is not signed in before the answer.
#[test]
#[cfg_attr(not(feature = "e2e"), ignore = "end-to-end: run with --features e2e")]
fn a_retry_of_a_started_setup_names_no_asset_and_reaches_no_server() {
    each_rail(Feature::OwnerSetup, |rail| {
        let server = server_key(rail, 1);
        let mut sandbox = Sandbox::new("setup-resume");
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
        // The retry asks again: a resume of the setup that was started before, with no asset.
        let retry = setup(false);
        let text = stderr(&retry);
        assert_eq!(code(&retry), Some(2), "{text}");
        assert!(text.contains("pass --yes"), "{text}");
        assert!(text.contains("Resume setup setup-0002"), "{text}");
        assert!(text.contains(&format!("on {}", rail.network)), "{text}");
        assert!(!text.contains("fund it with"), "{text}");
        mock.assert_untouched();
    });
}
