//! `allowit POLICY_ID help`: the agent's next commands for one policy, from the server's
//! public facts. Every value comes from the registered rail, so no test names a vendor.
mod common;

use common::{Mock, Sandbox, code, every_rail, stderr, stdout};
use serde_json::{Value, json};

const ID: &str = "Xg7IxRXCr2MhVdRrTFoYKeAw2iV4u463";
const ENDPOINT: &str = "https://rpc.example/v1";

/// Runs `args` in `sandbox` against a server that answers `replies` in order, and returns the
/// output with the request line of each request.
fn served(
    sandbox: &mut Sandbox,
    args: &[&str],
    replies: Vec<(u16, Value)>,
) -> (std::process::Output, Vec<String>) {
    let mock = Mock::new();
    sandbox.set("ALLOWIT_URL", &mock.url);
    let url = mock.url.clone();
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        replies
            .into_iter()
            .map(|(status, body)| {
                let (mut conn, _) = listener.accept().unwrap();
                let (head, _) = common::read_request(&mut conn);
                common::reply(&mut conn, status, &body);
                head.lines().next().unwrap_or_default().to_owned()
            })
            .collect::<Vec<_>>()
    });
    let output = sandbox.run(args);
    // A command that failed before its request leaves the server waiting.
    if code(&output) != Some(0) {
        let _ = std::net::TcpStream::connect(url.trim_start_matches("http://"));
    }
    (output, server.join().unwrap_or_default())
}

fn facts(net: &str, state: &str) -> Value {
    json!({
        "version": 1, "policyId": ID, "state": state, "network": net, "networkLabel": "Example",
        "testnet": true, "endpoint": ENDPOINT, "walletKind": "standard", "agentPublicKey": null,
        "serverPublicKey": null, "aLaterField": {"ignored": true},
    })
}

#[test]
fn a_draft_tells_the_agent_to_make_its_key_and_report_it() {
    every_rail(|rail| {
        let net = rail.network;
        let mut sandbox = Sandbox::new("policy-help-draft");
        let mut body = facts(net, "awaiting_agent_key");
        body["serverPublicKey"] = json!(rail.account(13).0);
        let (output, requests) = served(&mut sandbox, &[ID, "help"], vec![(200, body.clone())]);
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        assert_eq!(
            requests,
            [format!("GET /api/agent/policy?id={ID} HTTP/1.1")]
        );
        // Progress goes to stderr; the steps are stdout.
        assert_eq!(
            stderr(&output),
            "Reading the policy from the Allowit server\n"
        );
        let text = stdout(&output);
        let origin = sandbox_url(&sandbox);
        assert!(
            text.starts_with(&format!(
                "Allowit policy {ID} on {origin}: awaiting agent key\n"
            )),
            "{text}"
        );
        for line in [
            format!("  allowit rail use {net} --rpc-url {ENDPOINT}"),
            "  allowit keys generate agent".into(),
            "  allowit keys use agent".into(),
            format!("ALLOWIT_URL={origin} allowit {ID} help"),
            "Never pay from your own account".into(),
            "More: allowit guide".into(),
        ] {
            assert!(text.contains(&line), "{line:?} is missing in\n{text}");
        }
        // A test network funds the key with its faucet; another network does not offer it.
        assert_eq!(
            text.contains("  allowit keys fund agent\n"),
            allowit_sdk::routing::info(net).unwrap().testnet
        );
        // No trust, import or payment command before the wallet exists.
        for absent in ["rail trust", "wallet import", "exec --op"] {
            assert!(!text.contains(absent), "{absent} in\n{text}");
        }
        // The same as data.
        let (output, _) = served(&mut sandbox, &["--json", ID, "help"], vec![(200, body)]);
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        assert!(output.stderr.is_empty(), "{}", stderr(&output));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["state"], "awaiting_agent_key");
        assert_eq!(value["policyId"], ID);
        assert_eq!(value["network"], net);
        assert_eq!(value["endpoint"], ENDPOINT);
        assert_eq!(
            value["steps"][0]["commands"][1],
            "allowit keys generate agent"
        );
        assert_eq!(value["guide"], "allowit guide");
        assert!(value["next"].as_str().unwrap().contains("public key"));
        // The command changed no setting and made no key.
        assert!(!sandbox.config().join("config.toml").exists());
        assert!(!sandbox.config().join("secrets").exists());
    });
}

fn sandbox_url(sandbox: &Sandbox) -> String {
    let shown = sandbox.json(&["config", "show"]);
    shown["engineUrl"].as_str().unwrap().to_owned()
}

#[test]
fn an_active_policy_prints_trust_import_and_payment_with_its_real_values() {
    every_rail(|rail| {
        let net = rail.network;
        let mut sandbox = Sandbox::new("policy-help-active");
        sandbox.ok(&["rail", "use", net]);
        let agent = sandbox.json(&["keys", "generate", "payer"])["address"]
            .as_str()
            .unwrap()
            .to_owned();
        let server_key = rail.account(13).0;
        let destination = rail.account(14).0;
        let wallet = rail.fixtures.wallet.clone();
        let mut body = facts(net, "active");
        body["agentPublicKey"] = json!(agent);
        body["serverPublicKey"] = json!(server_key);
        body["wallet"] = json!(wallet);
        body["assetSymbol"] = json!("UNIT");
        body["limits"] = json!([{"key":"payment_limit","label":"Payment limit","value":"1 UNIT"}]);
        body["profile"] =
            json!({"portable":{"params":{"destinations":{"List":[{"Addr":destination}]}}}});
        let (output, _) = served(&mut sandbox, &[ID, "help"], vec![(200, body.clone())]);
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        let text = stdout(&output);
        let origin = sandbox_url(&sandbox);
        let expected = [
            format!("Allowit policy {ID} on {origin}: active"),
            "If the two differ, stop.".into(),
            format!("  allowit rail use {net} --rpc-url {ENDPOINT}"),
            format!("  allowit rail trust {origin} {server_key}"),
            // The local key that holds the agent public key, by its own name.
            "  allowit keys use payer".into(),
            format!("  ALLOWIT_URL={origin} allowit wallet import --policy {ID}"),
            format!("  allowit wallet use {wallet}"),
            format!(
                "  ALLOWIT_URL={origin} allowit exec --op transfer --to {destination} --amount UNIT:AMOUNT --request-id pay-0001"
            ),
            format!("  ALLOWIT_URL={origin} allowit status pay-0001 --wait 60s"),
            "  allowit history".into(),
            "Payment limit".into(),
            "1 UNIT".into(),
            "Exit 20 means the policy denied the request.".into(),
        ];
        let mut at = 0;
        for line in &expected {
            let found = text[at..].find(line.as_str());
            assert!(
                found.is_some(),
                "{line:?} is missing or out of order in\n{text}"
            );
            at += found.unwrap();
        }
        // A config directory named on this call stays on every printed command.
        let named = sandbox.config().to_string_lossy().into_owned();
        let (output, _) = served(
            &mut sandbox,
            &["--config-dir", &named, "--json", ID, "help"],
            vec![(200, body.clone())],
        );
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let commands: Vec<&str> = value["steps"][0]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .chain(value["examples"].as_object().unwrap().values())
            .map(|command| command.as_str().unwrap())
            .collect();
        assert_eq!(commands.len(), 8);
        for command in commands {
            assert!(
                command.contains(&format!("allowit --config-dir {named} ")),
                "{command}"
            );
        }
        // It pinned no server key and imported nothing.
        let config = std::fs::read_to_string(sandbox.config().join("config.toml")).unwrap();
        assert!(!config.contains(&server_key), "{config}");
        assert!(!sandbox.config().join("public-profiles").exists());
        // A device without the agent's key gets no setup and no payment command.
        let mut other = Sandbox::new("policy-help-other-device");
        let (output, _) = served(
            &mut other,
            &["--json", ID, "help"],
            vec![(200, body.clone())],
        );
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["localKey"], Value::Null);
        assert_eq!(value["steps"], json!([]));
        assert_eq!(value["examples"], json!({}));
        assert!(value["next"].as_str().unwrap().contains("cannot pay"));
        // A server value that a shell would interpret never reaches a command line.
        for (field, hostile) in [
            ("endpoint", json!("https://rpc.example/$(id)")),
            ("endpoint", json!("https://rpc.example/ ; id")),
            ("endpoint", json!("file:///etc/passwd")),
            ("serverPublicKey", json!("not-a-key; id")),
            ("wallet", json!("`id`")),
            ("assetSymbol", json!("A;id")),
            ("walletKind", json!("x y")),
            ("state", json!("unknown-state")),
            ("network", json!("unknown:network")),
            (
                "limits",
                json!([{"key":"k","label":"line\nbreak","value":"1"}]),
            ),
        ] {
            let mut bad = body.clone();
            bad[field] = hostile.clone();
            let (output, _) = served(&mut sandbox, &[ID, "help"], vec![(200, bad)]);
            assert!(
                matches!(code(&output), Some(3 | 5)),
                "{field}={hostile}: {:?} {}",
                code(&output),
                stderr(&output)
            );
            assert!(output.stdout.is_empty(), "{field}: {}", stdout(&output));
        }
    });
}

#[test]
fn an_unknown_id_a_missing_route_and_a_silent_server_have_their_own_exit_codes() {
    every_rail(|rail| {
        let net = rail.network;
        let mut sandbox = Sandbox::new("policy-help-errors");
        let unknown =
            json!({"error":"This server has no policy with this id.","code":"policy_unknown"});
        let (output, _) = served(&mut sandbox, &[ID, "help"], vec![(404, unknown)]);
        assert_eq!(code(&output), Some(4), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("error: policy_unknown: "),
            "{}",
            stderr(&output)
        );
        // A server of an earlier release has no such route.
        let (output, _) = served(
            &mut sandbox,
            &[ID, "help"],
            vec![(404, json!({"error":"Endpoint not found."}))],
        );
        assert_eq!(code(&output), Some(4), "{}", stderr(&output));
        assert!(stderr(&output).contains("does not serve policy help"));
        let (output, _) = served(&mut sandbox, &[ID, "help"], vec![(503, json!({}))]);
        assert_eq!(code(&output), Some(7), "{}", stderr(&output));
        // No server at all.
        sandbox.set("ALLOWIT_URL", "http://127.0.0.1:1");
        let output = sandbox.run(&[ID, "help"]);
        assert_eq!(code(&output), Some(7), "{}", stderr(&output));
        // JSON errors keep the one-object shape.
        let output = sandbox.run(&["--json", ID, "help"]);
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["exit"], 7);
        // A word that is no policy id is an unknown command, as before.
        let output = sandbox.run(&["short", "help"]);
        assert_eq!(code(&output), Some(2));
        // The hidden command checks the id that it is given.
        let output = sandbox.run(&["policy-help", "--id", "a b"]);
        assert_eq!(code(&output), Some(2), "{}", stderr(&output));
        let _ = net;
    });
}

#[test]
fn rail_use_keeps_the_endpoint_and_wallet_import_reads_the_profile_of_a_policy() {
    every_rail(|rail| {
        let net = rail.network;
        let mut sandbox = Sandbox::new("policy-help-import");
        let shown = sandbox.json(&["rail", "use", net, "--rpc-url", ENDPOINT]);
        assert_eq!(shown, json!({"rail": net, "rpcUrl": ENDPOINT}));
        let config = std::fs::read_to_string(sandbox.config().join("config.toml")).unwrap();
        assert!(config.contains(ENDPOINT), "{config}");
        // Without the flag the endpoint stays.
        sandbox.ok(&["rail", "use", net]);
        let config = std::fs::read_to_string(sandbox.config().join("config.toml")).unwrap();
        assert!(config.contains(ENDPOINT), "{config}");
        for bad in ["ftp://rpc.example", "rpc.example", "https://"] {
            let output = sandbox.run(&["rail", "use", net, "--rpc-url", bad]);
            assert_eq!(code(&output), Some(2), "{bad}");
        }
        // The import needs the pinned server key before it asks the server anything.
        let output = sandbox.run(&["wallet", "import", "--policy", ID]);
        assert_eq!(code(&output), Some(3), "{}", stderr(&output));
        assert!(stderr(&output).contains("no trusted server key"));
        // With the pin, a policy that is not active has no profile to import.
        let mock_origin = {
            let (output, _) = served(&mut sandbox, &["config", "show", "--json"], vec![]);
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            value["engineUrl"].as_str().unwrap().to_owned()
        };
        let _ = mock_origin;
        let mock = Mock::new();
        sandbox.set("ALLOWIT_URL", &mock.url);
        sandbox.ok(&["rail", "trust", &mock.url, &rail.account(13).0]);
        let listener = mock.listener;
        let body = facts(net, "draft");
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let (head, _) = common::read_request(&mut conn);
            common::reply(&mut conn, 200, &body);
            head.lines().next().unwrap_or_default().to_owned()
        });
        let output = sandbox.run(&["wallet", "import", "--policy", ID]);
        assert_eq!(
            server.join().unwrap(),
            format!("GET /api/agent/policy?id={ID} HTTP/1.1")
        );
        assert_eq!(code(&output), Some(3), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("not active yet"),
            "{}",
            stderr(&output)
        );
        // A file and a policy id together, or neither, are usage errors.
        let output = sandbox.run(&["wallet", "import", "profile.json", "--policy", ID]);
        assert_eq!(code(&output), Some(2));
        assert_eq!(code(&sandbox.run(&["wallet", "import"])), Some(2));
    });
}
