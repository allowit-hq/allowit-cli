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
    let (output, requests) = served_full(sandbox, args, replies);
    (
        output,
        requests
            .into_iter()
            .map(|(head, _)| head.lines().next().unwrap_or_default().to_owned())
            .collect(),
    )
}

/// The key that signed the agent request `head` and `body` for `path` on `net`, after the
/// rail verified its request proof.
fn proof_signer(net: &str, head: &str, body: &Value, path: &str) -> String {
    let bearer = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("authorization")
                .then(|| value.trim().strip_prefix("Bearer ").map(str::to_owned))
                .flatten()
        })
        .expect("an agent request carries a bearer");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    allowit_sdk::runtime::verify_request_proof(
        net,
        &bearer,
        now,
        path,
        &serde_json_canonicalizer::to_vec(body).unwrap(),
    )
    .expect("a valid request proof")
}

/// The agent's answer to the signed policy read: the public facts with what only the agent
/// reads.
fn mine(public: &Value, agent: &str) -> Value {
    let mut facts = public.clone();
    facts["agentPublicKey"] = json!(agent);
    facts
}

/// The server's refusal of a key that is not the policy's agent key.
fn not_agent() -> (u16, Value) {
    (
        403,
        json!({"error":"This key is not the agent key of this policy.","code":"not_policy_agent"}),
    )
}

/// As [`served`], with the head and the body of each request.
fn served_full(
    sandbox: &mut Sandbox,
    args: &[&str],
    replies: Vec<(u16, Value)>,
) -> (std::process::Output, Vec<(String, Value)>) {
    let mock = Mock::new();
    sandbox.set("ALLOWIT_URL", &mock.url);
    let url = mock.url.clone();
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        replies
            .into_iter()
            .map(|(status, body)| {
                let (mut conn, _) = listener.accept().unwrap();
                let request = common::read_request(&mut conn);
                common::reply(&mut conn, status, &body);
                request
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

/// The public facts of a policy: what anyone with its id reads.
fn facts(net: &str, state: &str) -> Value {
    json!({
        "version": 1, "policyId": ID, "state": state, "network": net, "networkLabel": "Example",
        "testnet": true, "endpoint": ENDPOINT, "walletKind": "standard",
        "serverPublicKey": null, "aLaterField": {"ignored": true},
    })
}

#[test]
#[cfg_attr(not(feature = "e2e"), ignore = "end-to-end: run with --features e2e")]
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
            "More: allowit guide".into(),
        ] {
            assert!(text.contains(&line), "{line:?} is missing in\n{text}");
        }
        // The agent makes a key and holds no account: its steps name no funding and no
        // account of its own, on a test network and on every other.
        for absent in ["keys fund", "your account", "own account"] {
            assert!(!text.contains(absent), "{absent} in\n{text}");
        }
        // No trust, import or payment command before the wallet exists.
        for absent in ["rail trust", "wallet import", "exec --op"] {
            assert!(!text.contains(absent), "{absent} in\n{text}");
        }
        // The same as data.
        let (output, _) = served(
            &mut sandbox,
            &["--json", ID, "help"],
            vec![(200, body.clone())],
        );
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
        // A device that already holds a key named `agent` on this rail uses it: no second
        // `keys generate agent`, and the public key is named.
        sandbox.ok(&["rail", "use", net]);
        let held = sandbox.json(&["keys", "generate", "agent"])["address"]
            .as_str()
            .unwrap()
            .to_owned();
        let (output, _) = served(&mut sandbox, &["--json", ID, "help"], vec![(200, body)]);
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let commands: Vec<&str> = value["steps"][0]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|command| command.as_str().unwrap())
            .collect();
        assert!(
            commands
                .iter()
                .all(|command| !command.contains("keys generate")),
            "{commands:?}"
        );
        assert!(commands.contains(&"allowit keys use agent"), "{commands:?}");
        assert!(value["next"].as_str().unwrap().contains(&held), "{value}");
    });
}

fn sandbox_url(sandbox: &Sandbox) -> String {
    let shown = sandbox.json(&["config", "show"]);
    shown["engineUrl"].as_str().unwrap().to_owned()
}

#[test]
#[cfg_attr(not(feature = "e2e"), ignore = "end-to-end: run with --features e2e")]
fn an_active_policy_prints_import_and_payment_to_its_agent_key_only() {
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
        let answers = |id: &str| {
            let mut public = facts(net, "active");
            public["policyId"] = json!(id);
            public["serverPublicKey"] = json!(server_key);
            public["walletState"] = json!("active");
            let mut theirs = mine(&public, &agent);
            theirs["wallet"] = json!(wallet);
            theirs["assetSymbol"] = json!("UNIT");
            theirs["limits"] =
                json!([{"key":"payment_limit","label":"Payment limit","value":"1 UNIT"}]);
            theirs["profile"] =
                json!({"portable":{"params":{"destinations":{"List":[{"Addr":destination}]}}}});
            theirs["ownerWords"] = json!([
                "Every Friday pay Ana 100.\nOnly Ana's address.",
                "Also pay Ben 150 on Fridays.",
            ]);
            (public, theirs)
        };
        let (public, body) = answers(ID);
        let both = || vec![(200, public.clone()), (200, body.clone())];
        let (output, requests) = served_full(&mut sandbox, &[ID, "help"], both());
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        // The public facts by the id alone, then the agent's read, signed with its key.
        assert_eq!(
            requests
                .iter()
                .map(|(head, _)| head.lines().next().unwrap())
                .collect::<Vec<_>>(),
            [
                format!("GET /api/agent/policy?id={ID} HTTP/1.1"),
                "POST /api/agent/policy HTTP/1.1".to_owned()
            ]
        );
        assert!(!requests[0].0.to_ascii_lowercase().contains("authorization"));
        assert_eq!(requests[1].1, json!({ "policyId": ID }));
        assert_eq!(
            proof_signer(net, &requests[1].0, &requests[1].1, "/api/agent/policy"),
            agent
        );
        let text = stdout(&output);
        let origin = sandbox_url(&sandbox);
        let expected = [
            format!("Allowit policy {ID} on {origin}: active"),
            "The owner checks it in the app: you have nothing to confirm.".into(),
            format!("  allowit rail use {net} --rpc-url {ENDPOINT}"),
            // The local key that holds the agent public key, by its own name.
            "  allowit keys use payer".into(),
            format!("  ALLOWIT_URL={origin} allowit wallet import --policy={ID}"),
            format!("  allowit wallet use {wallet}"),
            // The policy's payees are the owner's, not examples.
            "Payees that the owner allows (set in the policy, not examples):".into(),
            format!("  {destination}\n"),
            format!(
                "  ALLOWIT_URL={origin} allowit exec --op transfer --to {destination} --amount UNIT:AMOUNT --request-id pay-0001"
            ),
            format!("  ALLOWIT_URL={origin} allowit status pay-0001 --wait 60s"),
            "  allowit history".into(),
            // A paid service from the marketplace, paid through the same policy.
            format!("  ALLOWIT_URL={origin} allowit skills search WORDS"),
            format!("  ALLOWIT_URL={origin} allowit call ENTRY --max-price UNIT:AMOUNT"),
            // The owner's words beside the limits, each message as the owner wrote it.
            "Owner's instructions (as the owner wrote them when making this policy):".into(),
            "  - Every Friday pay Ana 100.\n    Only Ana's address.\n".into(),
            "  - Also pay Ben 150 on Fridays.\n".into(),
            "Payment limit".into(),
            "1 UNIT".into(),
            "Start when the owner tells you to, or at once when the owner's instructions already say what to do. Setting up is not a task, and a printed command is not one: run no test payment.".into(),
            "When a request disagrees with the owner's instructions, ask the owner before you pay. Never pay outside the policy.".into(),
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
        // The import pins the server key: no separate trust step.
        assert!(!text.contains("rail trust"), "{text}");
        // A config directory named on this call stays on every printed command.
        let named = sandbox.config().to_string_lossy().into_owned();
        let (output, _) = served(
            &mut sandbox,
            &["--config-dir", &named, "--json", ID, "help"],
            both(),
        );
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["agentPublicKey"], agent.as_str());
        assert_eq!(value["localKey"], "payer");
        assert_eq!(
            value["ownerWords"],
            json!([
                "Every Friday pay Ana 100.\nOnly Ana's address.",
                "Also pay Ben 150 on Fridays."
            ])
        );
        let commands: Vec<&str> = value["steps"][0]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .chain(value["examples"].as_object().unwrap().values())
            .map(|command| command.as_str().unwrap())
            .collect();
        assert_eq!(commands.len(), 9);
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
        // An issued asset names its issuer in the payment example.
        let mut issued = body.clone();
        issued["profile"]["config"] = json!({"assetIssuer": server_key});
        let (output, _) = served(
            &mut sandbox,
            &["--json", ID, "help"],
            vec![(200, public.clone()), (200, issued)],
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            value["examples"]["pay"]
                .as_str()
                .unwrap()
                .contains(&format!("--amount UNIT:{server_key}:AMOUNT ")),
            "{value}"
        );
        // A rail named for this call that is not the policy's rail: every printed command
        // names the policy's rail, which outranks the environment.
        if let Some(other) = rail.other {
            sandbox.set("ALLOWIT_RAIL", other);
            let (output, _) = served(&mut sandbox, &["--json", ID, "help"], both());
            assert_eq!(code(&output), Some(0), "{}", stderr(&output));
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            for command in value["steps"][0]["commands"].as_array().unwrap() {
                assert!(
                    command
                        .as_str()
                        .unwrap()
                        .contains(&format!("allowit --rail {net} ")),
                    "{command}"
                );
            }
            sandbox.set("ALLOWIT_RAIL", net);
            let (output, _) = served(&mut sandbox, &["--json", ID, "help"], both());
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(
                !value["steps"][0]["commands"][1]
                    .as_str()
                    .unwrap()
                    .contains("--rail")
            );
        }
        // A policy id that starts with a hyphen keeps its attached form on the import.
        let hyphen = "-1WagPbql_eK8CwzC4dxHJDMtC5wilVg";
        let (hyphen_public, hyphen_body) = answers(hyphen);
        let (output, requests) = served(
            &mut sandbox,
            &["--json", hyphen, "help"],
            vec![(200, hyphen_public), (200, hyphen_body)],
        );
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        assert_eq!(
            requests[0],
            format!("GET /api/agent/policy?id={hyphen} HTTP/1.1")
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            value["steps"][0]["commands"][2]
                .as_str()
                .unwrap()
                .ends_with(&format!("wallet import --policy={hyphen}"))
        );
        let import = sandbox.run(&["wallet", "import", &format!("--policy={hyphen}")]);
        assert_ne!(code(&import), Some(2), "{}", stderr(&import));
        // An answer for another policy or another key than the one that signed is refused.
        for (field, wrong) in [
            ("policyId", json!("another-policy-id")),
            ("agentPublicKey", json!(server_key)),
        ] {
            let mut wrong_body = body.clone();
            wrong_body[field] = wrong;
            let (output, _) = served(
                &mut sandbox,
                &[ID, "help"],
                vec![(200, public.clone()), (200, wrong_body)],
            );
            assert_eq!(code(&output), Some(5), "{field}: {}", stderr(&output));
            assert!(output.stdout.is_empty());
        }
        // The JSON form carries the owner's check of the server key too.
        let (output, _) = served(&mut sandbox, &["--json", ID, "help"], both());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            value["serverKeyCheck"],
            "The CLI pins this server key when it imports the wallet, and checks it before each signature. The owner checks it in the app: you have nothing to confirm."
        );
        assert_eq!(value["allowedPayees"], json!([destination]));
        // An IPv6 endpoint is one quoted shell word.
        let mut v6 = public.clone();
        v6["endpoint"] = json!("https://[2001:db8::1]:8443/rpc");
        let (output, _) = served(
            &mut sandbox,
            &["--json", ID, "help"],
            vec![(200, v6), (200, body.clone())],
        );
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            value["steps"][0]["commands"][0]
                .as_str()
                .unwrap()
                .ends_with("--rpc-url 'https://[2001:db8::1]:8443/rpc'"),
            "{value}"
        );
        // A wallet named for this call that is not the policy wallet: the payment names the
        // policy wallet, which outranks the environment.
        sandbox.set("ALLOWIT_WALLET", &rail.fixtures.account);
        let (output, _) = served(&mut sandbox, &["--json", ID, "help"], both());
        if code(&output) == Some(0) {
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(
                value["examples"]["pay"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("allowit --wallet {wallet} exec ")),
                "{value}"
            );
        }
        sandbox.set("ALLOWIT_WALLET", "");
        // A device without a local key reads the public facts only: no setup and no payment.
        let mut other = Sandbox::new("policy-help-other-device");
        let (output, requests) = served(
            &mut other,
            &["--json", ID, "help"],
            vec![(200, public.clone())],
        );
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        assert_eq!(requests.len(), 1);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["localKey"], Value::Null);
        assert_eq!(value["agentPublicKey"], Value::Null);
        assert_eq!(value["wallet"], Value::Null);
        assert_eq!(value["limits"], json!([]));
        assert_eq!(value["ownerWords"], json!([]));
        assert_eq!(value["allowedPayees"], json!([]));
        assert_eq!(value["steps"], json!([]));
        assert_eq!(value["examples"], json!({}));
        assert!(value["next"].as_str().unwrap().contains("cannot pay"));
        // A device whose key the server does not admit as the agent: the same.
        other.ok(&["rail", "use", net]);
        other.ok(&["keys", "generate", "agent"]);
        let (output, requests) = served(
            &mut other,
            &["--json", ID, "help"],
            vec![(200, public.clone()), not_agent()],
        );
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        assert_eq!(requests.len(), 2);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["localKey"], Value::Null);
        assert_eq!(value["steps"], json!([]));
        assert!(value["next"].as_str().unwrap().contains("cannot pay"));
        // A server value that a shell would interpret never reaches a command line.
        for (field, hostile, in_public) in [
            ("endpoint", json!("https://rpc.example/$(id)"), true),
            ("endpoint", json!("https://rpc.example/ ; id"), true),
            ("endpoint", json!("file:///etc/passwd"), true),
            ("serverPublicKey", json!("not-a-key; id"), true),
            ("walletKind", json!("x y"), true),
            ("state", json!("unknown-state"), true),
            ("network", json!("unknown:network"), true),
            ("wallet", json!("`id`"), false),
            ("assetSymbol", json!("A;id"), false),
            (
                "limits",
                json!([{"key":"k","label":"line\nbreak","value":"1"}]),
                false,
            ),
            ("ownerWords", json!(["x".repeat(3_001)]), false),
            ("ownerWords", json!(vec!["x"; 33]), false),
        ] {
            let (mut bad_public, mut bad_body) = (public.clone(), body.clone());
            if in_public {
                bad_public[field] = hostile.clone();
                bad_body[field] = hostile.clone();
            } else {
                bad_body[field] = hostile.clone();
            }
            let (output, _) = served(
                &mut sandbox,
                &[ID, "help"],
                vec![(200, bad_public), (200, bad_body)],
            );
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

/// The help says what one request costs the policy owner in credits and that the agent needs
/// none. When the owner cannot pay, it says what the refusal will be.
#[test]
#[cfg_attr(not(feature = "e2e"), ignore = "end-to-end: run with --features e2e")]
fn the_help_shows_the_credits_that_each_request_costs_the_owner() {
    every_rail(|rail| {
        let net = rail.network;
        let mut sandbox = Sandbox::new("policy-help-credits");
        sandbox.ok(&["rail", "use", net]);
        let agent = sandbox.json(&["keys", "generate", "agent"])["address"]
            .as_str()
            .unwrap()
            .to_owned();
        // Only the policy's agent reads the credits: they are in its answer, not the public one.
        let public = facts(net, "draft");
        let mut body = mine(&public, &agent);
        body["credits"] = json!({"evaluation": "1.5", "tariff": "2026-11-14", "ownerCanPay": true});
        let (output, _) = served(
            &mut sandbox,
            &[ID, "help"],
            vec![(200, public.clone()), (200, body.clone())],
        );
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        let text = stdout(&output);
        assert!(
            text.contains("1.5 per request, charged to the owner (price list 2026-11-14)\n"),
            "{text}"
        );
        assert!(
            text.contains("Credits: each request that you sign costs the policy owner 1.5 credits, whatever its outcome, and the model tokens that it uses, in whole credits. You need no credits of your own.\n"),
            "{text}"
        );
        assert!(!text.contains("credits_exhausted"), "{text}");
        let (output, _) = served(
            &mut sandbox,
            &["--json", ID, "help"],
            vec![(200, public.clone()), (200, body.clone())],
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            value["credits"],
            json!({"evaluation": "1.5", "tariff": "2026-11-14", "paidBy": "owner", "ownerCanPay": true})
        );
        // An owner without credits: the agent learns what the refusal will be and whom to tell.
        body["credits"]["ownerCanPay"] = json!(false);
        let (output, _) = served(
            &mut sandbox,
            &[ID, "help"],
            vec![(200, public.clone()), (200, body.clone())],
        );
        let text = stdout(&output);
        assert!(
            text.contains("The owner has no credits left now: Allowit refuses each request with `credits_exhausted` until the owner gets credits. Tell the owner."),
            "{text}"
        );
        // An answer without credits: the help has no such line.
        let (output, _) = served(
            &mut sandbox,
            &[ID, "help"],
            vec![(200, public.clone()), (200, mine(&public, &agent))],
        );
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        assert!(!stdout(&output).contains("redits"), "{}", stdout(&output));
        // Credits that are no plain amount are refused, never printed.
        body["credits"]["evaluation"] = json!("1.5\u{1b}[31m");
        let (output, _) = served(
            &mut sandbox,
            &[ID, "help"],
            vec![(200, public.clone()), (200, body)],
        );
        assert_eq!(code(&output), Some(5), "{}", stderr(&output));
        assert!(!stdout(&output).contains("per request"));
    });
}

/// A device with more local keys than the help tries says so: it names no missing key and
/// tells the agent to select the key that it gave the owner.
#[test]
#[cfg_attr(not(feature = "e2e"), ignore = "end-to-end: run with --features e2e")]
fn a_search_that_leaves_keys_untried_says_to_select_the_agent_key() {
    every_rail(|rail| {
        let net = rail.network;
        let mut sandbox = Sandbox::new("policy-help-many-keys");
        sandbox.ok(&["rail", "use", net]);
        for n in 1..=9 {
            sandbox.ok(&["keys", "generate", &format!("key-{n}")]);
        }
        let mut public = facts(net, "active");
        public["serverPublicKey"] = json!(rail.account(13).0);
        let mut replies = vec![(200, public)];
        replies.extend((0..8).map(|_| not_agent()));
        let (output, requests) = served(&mut sandbox, &["--json", ID, "help"], replies);
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        // The public read and eight signed reads, no more.
        assert_eq!(requests.len(), 9);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["steps"], json!([]));
        let next = value["next"].as_str().unwrap();
        assert!(next.contains("more than 8 keys"), "{next}");
        assert!(next.contains("keys use NAME"), "{next}");
        assert!(!next.contains("cannot pay"), "{next}");
    });
}

#[test]
#[cfg_attr(not(feature = "e2e"), ignore = "end-to-end: run with --features e2e")]
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
#[cfg_attr(not(feature = "e2e"), ignore = "end-to-end: run with --features e2e")]
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
        // A policy that is not active has no profile to import, with or without a pin.
        let server_key = rail.account(13).0;
        let mut draft = facts(net, "draft");
        draft["serverPublicKey"] = json!(server_key);
        let (output, requests) = served(
            &mut sandbox,
            &["wallet", "import", "--policy", ID],
            vec![(200, draft)],
        );
        assert_eq!(
            requests,
            [format!("GET /api/agent/policy?id={ID} HTTP/1.1")]
        );
        assert_eq!(code(&output), Some(3), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("not active yet"),
            "{}",
            stderr(&output)
        );
        let mut active = facts(net, "active");
        active["serverPublicKey"] = json!(server_key);
        // A server that names another key than the one pinned for it is refused before the
        // agent's read: nothing is imported and the pin stays.
        let pinned = rail.account(15).0;
        let mock = Mock::new();
        sandbox.set("ALLOWIT_URL", &mock.url);
        sandbox.ok(&["rail", "trust", &mock.url, &pinned]);
        let listener = mock.listener;
        let reply = active.clone();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let (head, _) = common::read_request(&mut conn);
            common::reply(&mut conn, 200, &reply);
            head.lines().next().unwrap_or_default().to_owned()
        });
        let output = sandbox.run(&["wallet", "import", "--policy", ID]);
        assert_eq!(
            server.join().unwrap(),
            format!("GET /api/agent/policy?id={ID} HTTP/1.1")
        );
        assert_eq!(code(&output), Some(4), "{}", stderr(&output));
        let error = stderr(&output);
        assert!(error.contains("error: server_key_differs: "), "{error}");
        assert!(
            error.contains(&server_key) && error.contains(&pinned),
            "{error}"
        );
        assert!(error.contains("Nothing was imported"), "{error}");
        let config = std::fs::read_to_string(sandbox.config().join("config.toml")).unwrap();
        assert!(!config.contains(&server_key), "{config}");
        // Without a pin and without a selected agent key the import stops after the public
        // facts: the profile needs the agent's read.
        let (output, requests) = served(
            &mut sandbox,
            &["wallet", "import", "--policy", ID],
            vec![(200, active.clone())],
        );
        assert_eq!(requests.len(), 1);
        assert_eq!(code(&output), Some(3), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("no agent key"),
            "{}",
            stderr(&output)
        );
        // A selected key that the server does not admit as the policy's agent reads nothing.
        sandbox.ok(&["keys", "generate", "agent"]);
        sandbox.ok(&["keys", "use", "agent"]);
        let (output, requests) = served_full(
            &mut sandbox,
            &["wallet", "import", "--policy", ID],
            vec![(200, active.clone()), not_agent()],
        );
        assert_eq!(requests.len(), 2);
        let agent = sandbox.json(&["keys", "show", "agent"])["address"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            proof_signer(net, &requests[1].0, &requests[1].1, "/api/agent/policy"),
            agent
        );
        assert_eq!(code(&output), Some(4), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("error: not_policy_agent: "),
            "{}",
            stderr(&output)
        );
        // A profile of another policy than the requested one is refused before any import.
        let mut theirs = mine(&active, &agent);
        theirs["profile"] = json!({"policyId": "another-policy-id", "network": net});
        let (output, _) = served(
            &mut sandbox,
            &["wallet", "import", "--policy", ID],
            vec![(200, active.clone()), (200, theirs)],
        );
        assert_eq!(code(&output), Some(5), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("profile of another policy"),
            "{}",
            stderr(&output)
        );
        assert!(!sandbox.config().join("public-profiles").exists());
        // No refused import pinned the server's key.
        let config = std::fs::read_to_string(sandbox.config().join("config.toml")).unwrap();
        assert!(!config.contains(&server_key), "{config}");
        // A file and a policy id together, or neither, are usage errors.
        let output = sandbox.run(&["wallet", "import", "profile.json", "--policy", ID]);
        assert_eq!(code(&output), Some(2));
        assert_eq!(code(&sandbox.run(&["wallet", "import"])), Some(2));
    });
}
