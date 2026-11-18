//! The checks before the agent key signs a fresh operation: the server key that co-signs it is
//! the key pinned for its server, and the policy, read as its agent, is active and names this
//! wallet and that key. Each refusal stops the command before any execution request, so
//! nothing is signed. Every value comes from the registered rail, so no test names a vendor.
mod common;

use allowit_sdk::plugins::Feature;
use common::{Mock, Sandbox, code, each_rail, stderr};
use serde_json::{Value, json};
use std::net::TcpListener;

/// Answers `replies` in order on `listener` and returns the request line and body of each.
fn answer(
    listener: TcpListener,
    replies: Vec<(u16, Value)>,
) -> std::thread::JoinHandle<Vec<(String, Value)>> {
    std::thread::spawn(move || {
        replies
            .into_iter()
            .map(|(status, body)| {
                let (mut conn, _) = listener.accept().unwrap();
                let (head, request) = common::read_request(&mut conn);
                common::reply(&mut conn, status, &body);
                (head.lines().next().unwrap_or_default().to_owned(), request)
            })
            .collect()
    })
}

#[test]
fn exec_checks_the_server_key_and_the_policy_before_the_agent_signs() {
    each_rail(Feature::Transfer, |rail| {
        let net = rail.network;
        let fixtures = rail.fixtures;
        // The rail's profile fixture, with an agent key that this device holds.
        let (agent, secret) = rail.account(31);
        let mut profile: Value = serde_json::from_slice(&fixtures.public_profile).unwrap();
        profile["config"]["agent"] = json!(agent);
        let (description, _) = rail
            .plugin
            .decode_profile(net, &serde_json::to_vec(&profile).unwrap())
            .unwrap();
        assert_eq!(description.agent, agent);
        let mut sandbox = Sandbox::new("presign");
        let mock = Mock::new();
        common::public_profile_fixture(&sandbox, &profile, &mock.url);
        sandbox.set("ALLOWIT_URL", &mock.url);
        let file = sandbox.private_file("agent.key", &secret);
        sandbox.ok(&["rail", "use", net, "--rpc-url", "http://127.0.0.1:1"]);
        sandbox.ok(&["keys", "import", "agent", file.to_str().unwrap()]);
        sandbox.ok(&["keys", "use", "agent"]);
        let amount = format!("{}:1", fixtures.token.code);
        let pay = [
            "exec",
            "--op",
            "transfer",
            "--to",
            fixtures.recipient.as_str(),
            "--amount",
            amount.as_str(),
            "--request-id",
            "pay-0001",
        ];
        // No key is pinned for the server: the import of the policy pins it.
        let output = sandbox.run(&pay);
        assert_eq!(code(&output), Some(3), "{}", stderr(&output));
        assert!(
            stderr(&output).contains(&format!(
                "run allowit wallet import --policy={}",
                description.policy_id
            )),
            "{}",
            stderr(&output)
        );
        mock.assert_untouched();
        // Another key is pinned for the server than the one that co-signs this policy's
        // operations: refused before any request.
        let other = rail.account(15).0;
        sandbox.ok(&["rail", "trust", &mock.url, &other]);
        let output = sandbox.run(&pay);
        assert_eq!(code(&output), Some(4), "{}", stderr(&output));
        let error = stderr(&output);
        assert!(error.contains("error: server_key_differs: "), "{error}");
        assert!(
            error.contains(&description.server) && error.contains(&other),
            "{error}"
        );
        assert!(error.contains("Nothing was signed"), "{error}");
        mock.assert_untouched();
        // The pinned key is the profile's: the agent reads its policy, signed, and the answer
        // decides before any execution request.
        sandbox.ok(&["rail", "trust", &mock.url, &description.server]);
        let facts = |state: &str, wallet: &str, server: &str| {
            json!({
                "version": 1, "policyId": description.policy_id, "state": state, "network": net,
                "networkLabel": "Example", "testnet": true, "endpoint": null,
                "walletKind": "standard", "serverPublicKey": server,
                "agentPublicKey": description.agent, "wallet": wallet,
            })
        };
        let refusal = json!({"error":"This key is not the agent key of this policy.","code":"not_policy_agent"});
        let listener = mock.listener;
        for (reply, refused) in [
            (
                (
                    200,
                    facts("inactive", &description.wallet, &description.server),
                ),
                "policy_not_active",
            ),
            (
                (
                    200,
                    facts("active", &fixtures.recipient, &description.server),
                ),
                "wallet_differs",
            ),
            (
                (200, facts("active", &description.wallet, &other)),
                "server_key_differs",
            ),
            ((403, refusal.clone()), "not_policy_agent"),
            // Everything that the server says holds: the ledger decides, and this endpoint
            // does not answer.
            (
                (
                    200,
                    facts("active", &description.wallet, &description.server),
                ),
                "wallet_unreadable",
            ),
        ] {
            let server = answer(listener.try_clone().unwrap(), vec![reply]);
            let output = sandbox.run(&pay);
            let requests = server.join().unwrap();
            // No other request followed: a check before signing evaluated nothing to charge.
            listener.set_nonblocking(true).unwrap();
            assert!(listener.accept().is_err(), "{refused}: a second request");
            listener.set_nonblocking(false).unwrap();
            assert_eq!(code(&output), Some(4), "{refused}: {}", stderr(&output));
            let error = stderr(&output);
            assert!(error.contains(&format!("error: {refused}: ")), "{error}");
            assert!(error.contains("Nothing was signed"), "{error}");
            // The one request is the agent's policy read: no execution request was sent.
            assert_eq!(
                requests,
                [(
                    "POST /api/agent/policy HTTP/1.1".to_owned(),
                    json!({"policyId": description.policy_id})
                )],
                "{refused}"
            );
        }
        // A build-only request signs nothing, so it reads no policy from the server.
        let mut built = pay.to_vec();
        built.push("--build-only");
        let output = sandbox.run(&built);
        assert!(
            !stderr(&output).contains("Nothing was signed"),
            "{}",
            stderr(&output)
        );
        listener.set_nonblocking(true).unwrap();
        assert!(
            listener.accept().is_err(),
            "a build-only request reached the server"
        );
    });
}
