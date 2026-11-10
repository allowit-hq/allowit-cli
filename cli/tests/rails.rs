//! Rail selection, keys and the public wallet profiles: the rail decides the
//! key curve, the address formats and the adapter, and a command never reads
//! a credential, a profile or the network that it does not need.
//!
//! Every test runs on each rail that the SDK registers. A test of a feature
//! that a rail does not serve is skipped for that rail by its own declaration.
//! No test names a vendor: every address, asset and profile comes from the
//! rail's fixtures.
mod common;

use allowit_sdk::plugins::Feature;
use common::{Mock, Rail, Sandbox, code, each_rail, every_rail, stderr, stdout};
use serde_json::{Value, json};

#[test]
fn unknown_rail_keeps_the_cli_recovery_hint_without_persisting_settings() {
    let sandbox = Sandbox::new("unknown-rail-hint");
    let mock = Mock::new();
    let output = sandbox.run(&["--json", "rail", "show", "example:unknown"]);
    assert_eq!(code(&output), Some(2));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(
        error["error"]["message"],
        "unknown rail \"example:unknown\"; run allowit rail ls"
    );
    assert_eq!(error["error"]["exit"], 2);
    assert!(!sandbox.config().join("config.toml").exists());
    assert!(!sandbox.config().join("keys").exists());
    assert!(!sandbox.config().join("secrets").exists());
    mock.assert_untouched();
}

#[test]
fn rail_ls_and_show_list_exactly_the_registered_rails() {
    let sandbox = Sandbox::new("rail-ls");
    let listed: Vec<String> = sandbox
        .json(&["rail", "ls"])
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["rail"].as_str().unwrap().to_owned())
        .collect();
    let registered: Vec<String> = allowit_sdk::plugins::rails()
        .iter()
        .flat_map(|p| p.networks().iter().map(|n| n.id.to_owned()))
        .collect();
    assert_eq!(listed, registered);
    every_rail(|rail| {
        let shown = sandbox.json(&["rail", "show", rail.network]);
        assert_eq!(shown["rail"], rail.network);
        let ops = rail.plugin.capabilities(rail.network).operations;
        assert_eq!(
            shown["ops"],
            json!(ops),
            "{}: rail show reports the plugin's operations",
            rail.network
        );
    });
}

#[test]
fn rail_aliases_and_settings_stay_bound_to_the_selected_network() {
    every_rail(|rail| {
        let mut sandbox = Sandbox::new("rail");
        assert_eq!(code(&sandbox.run(&["keys", "generate", "agent"])), Some(3));
        for info in rail.plugin.networks() {
            for alias in info.aliases {
                assert_eq!(sandbox.json(&["rail", "use", alias])["rail"], info.id);
            }
        }
        sandbox.ok(&["rail", "use", rail.network]);
        let wallet = rail.fixtures.wallet.as_str();
        sandbox.ok(&["wallet", "use", wallet]);
        if let Some(other) = rail.other {
            sandbox.ok(&["rail", "use", other]);
            let config = sandbox.json(&["config", "show"]);
            assert!(config["config"]["rails"][other]["wallet"].is_null());
            assert_eq!(config["config"]["rails"][rail.network]["wallet"], wallet);
            assert_eq!(config["railSource"], "config.toml");
            assert_eq!(
                sandbox.json(&["--rail", other, "rail", "show"])["rail"],
                other
            );
        }
        assert_eq!(
            code(&sandbox.run(&["rail", "use", "example:unknown"])),
            Some(2)
        );
        sandbox.set("ALLOWIT_RAIL", rail.network);
        assert_eq!(sandbox.json(&["rail", "show"])["rail"], rail.network);
    });
}

#[test]
fn keys_keep_private_material_out_of_public_metadata() {
    every_rail(keys_on);
}

fn keys_on(rail: &Rail<'_>) {
    let net = rail.network;
    let sandbox = Sandbox::new("keys");
    sandbox.ok(&["rail", "use", net]);
    let generated = sandbox.json(&["--rail", net, "keys", "generate", "agent"]);
    let caps = rail.plugin.capabilities(net);
    assert_eq!(generated["curve"], caps.curve);
    let address = generated["address"].as_str().unwrap().to_owned();
    if caps.address_is_key {
        assert_eq!(generated["publicKey"], generated["address"]);
    }
    // Private keys stay in secrets/, mode 0600 in a 0700 directory.
    let secret = sandbox.ok(&["keys", "export", "agent"]);
    let secret = secret.trim();
    assert!(!secret.is_empty());
    let metadata = std::fs::read_to_string(sandbox.config().join("keys/agent.toml")).unwrap();
    assert!(!metadata.contains(secret));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode =
            |p: std::path::PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(sandbox.config().join("secrets")), 0o700);
        assert_eq!(mode(sandbox.config().join("secrets/agent")), 0o600);
    }
    let listed = sandbox.json(&["keys", "ls"]);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(!listed.to_string().contains(secret));
    sandbox.ok(&["keys", "use", "agent"]);
    assert_eq!(
        sandbox.json(&["config", "show"])["config"]["rails"][net]["agent"],
        "agent"
    );
    if let Some(other) = rail.other {
        assert_eq!(
            code(&sandbox.run(&["--rail", other, "keys", "use", "agent"])),
            Some(0)
        );
    }
    // Re-import of the exported key under another name; the address is the same.
    let file = sandbox.private_file("agent.key", secret);
    let copy = sandbox.json(&[
        "--rail",
        net,
        "keys",
        "import",
        "copy",
        file.to_str().unwrap(),
    ]);
    assert_eq!(copy["address"], address);
    // A seed-derived secret imports to the plugin's address for that seed.
    let (expected, derived) = rail.account(11);
    let file = sandbox.private_file("derived.key", &derived);
    let imported = sandbox.json(&["keys", "import", "derived", file.to_str().unwrap()]);
    assert_eq!(imported["address"], expected);
    sandbox.ok(&["keys", "rm", "derived", "--yes"]);
    // keys add records a public address that cannot sign.
    let owner = sandbox.json(&[
        "--rail",
        net,
        "keys",
        "add",
        "owner",
        "--public-key",
        &address,
    ]);
    assert_eq!(owner["secret"], false);
    assert_eq!(code(&sandbox.run(&["keys", "use", "owner"])), Some(3));
    let bad = sandbox.run(&[
        "--rail",
        net,
        "keys",
        "add",
        "bad",
        "--public-key",
        &rail.fixtures.invalid_address,
    ]);
    assert_eq!(code(&bad), Some(2));
    let output = sandbox.run(&["keys", "export", "owner"]);
    assert_eq!(code(&output), Some(3));
    // Removal is destructive: it needs --yes without a terminal.
    let output = sandbox.run(&["keys", "rm", "copy"]);
    assert_eq!(code(&output), Some(2));
    assert!(stderr(&output).contains("--yes"));
    sandbox.ok(&["keys", "rm", "copy", "--yes"]);
    assert!(!sandbox.config().join("secrets/copy").exists());
    assert_eq!(sandbox.json(&["keys", "ls"]).as_array().unwrap().len(), 2);
    // A broken metadata file does not hide the other keys.
    std::fs::write(sandbox.config().join("keys/bad.toml"), "garbage = [").unwrap();
    let output = sandbox.run(&["--json", "keys", "ls"]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    let listed: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert!(
        stderr(&output).starts_with("invalid  "),
        "{}",
        stderr(&output)
    );
    assert!(stderr(&output).contains("bad.toml"));
}

#[test]
fn concurrent_commands_keep_every_config_edit_and_one_owner_per_key_name() {
    every_rail(|rail| {
        let net = rail.network;
        let sandbox = Sandbox::new("concurrent");
        let spawn_all = |args: &dyn Fn(u8) -> Vec<String>| {
            let children: Vec<_> = (1..=8u8)
                .map(|n| {
                    let child = sandbox
                        .command()
                        .args(args(n))
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .unwrap();
                    (n, child)
                })
                .collect();
            children
                .into_iter()
                .map(|(n, mut child)| (n, child.wait().unwrap().code()))
                .collect::<Vec<_>>()
        };
        // Each wallet use reads and rewrites config.toml; none loses another's wallet.
        let wallet = |n: u8| rail.account(n).0;
        for (_, exit) in spawn_all(&|n| {
            ["--rail", net, "wallet", "use", &wallet(n)]
                .map(Into::into)
                .into()
        }) {
            assert_eq!(exit, Some(0));
        }
        let config = sandbox.json(&["config", "show"]);
        let mut wallets: Vec<_> = config["config"]["rails"][net]["wallets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_str().unwrap().to_owned())
            .collect();
        wallets.sort();
        let mut expected = (1..=8).map(wallet).collect::<Vec<_>>();
        expected.sort();
        assert_eq!(wallets, expected);
        // Only one keys add claims a name; the stored address is the winner's.
        let owner = |n: u8| rail.account(100 + n).0;
        let results = spawn_all(&|n| {
            [
                "--rail",
                net,
                "keys",
                "add",
                "owner",
                "--public-key",
                &owner(n),
            ]
            .map(Into::into)
            .into()
        });
        let winners: Vec<_> = results
            .iter()
            .filter(|(_, exit)| *exit == Some(0))
            .collect();
        assert_eq!(winners.len(), 1, "{results:?}");
        assert!(
            results.iter().all(|(_, exit)| matches!(exit, Some(0 | 3))),
            "{results:?}"
        );
        assert_eq!(
            sandbox.json(&["keys", "show", "owner"])["address"],
            owner(winners[0].0)
        );
    });
}

#[test]
fn faucet_funding_goes_through_the_rail_plugin_and_its_endpoint() {
    each_rail(Feature::KeysFund, |rail| {
        let mut sandbox = Sandbox::new("fund");
        let mock = Mock::new();
        sandbox.set("ALLOWIT_FAUCET_URL", &mock.url);
        sandbox.ok(&["rail", "use", rail.network]);
        let key = sandbox.json(&["keys", "generate", "agent"]);
        let address = key["address"].as_str().unwrap().to_owned();
        let listener = mock.listener;
        let expected = address.clone();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let (head, body) = common::read_request(&mut conn);
            // The plugin owns the wire format; the request names the key.
            assert!(
                head.contains(&expected) || body.to_string().contains(&expected),
                "{head}"
            );
            common::reply(&mut conn, 503, &json!({"error": "unavailable"}));
        });
        let output = sandbox.run(&["keys", "fund", "agent"]);
        assert_ne!(code(&output), Some(0), "a refused faucet is not funding");
        assert!(!stdout(&output).contains("transactionHash"));
        server.join().unwrap();
        // A network that is not a test network has no faucet.
        for info in rail.plugin.networks().iter().filter(|n| !n.testnet) {
            let net = info.id;
            let output = sandbox.run(&["--rail", net, "keys", "fund", "agent"]);
            assert_eq!(code(&output), Some(3), "{net}: {}", stderr(&output));
        }
    });
}

#[test]
fn invalid_input_fails_before_any_settings_bundle_or_network_access() {
    each_rail(Feature::Transfer, |rail| {
        let net = rail.network;
        let mut sandbox = Sandbox::new("cross");
        let mock = Mock::new();
        sandbox
            .set("ALLOWIT_URL", &mock.url)
            .set("ALLOWIT_PROFILE_UNSUPPORTED", "/must-not-be-read");
        let fixtures = rail.fixtures;
        let amount = format!("{}:1", fixtures.token.code);
        for (to, amount, needle) in [
            (
                fixtures.invalid_address.as_str(),
                amount.as_str(),
                "recipient",
            ),
            (fixtures.recipient.as_str(), "NOPE:1", "NOPE"),
        ] {
            let args = [
                "--rail", net, "exec", "--op", "transfer", "--to", to, "--amount", amount,
            ];
            let output = sandbox.run(&args);
            assert_eq!(code(&output), Some(2), "{args:?}: {}", stderr(&output));
            assert!(
                stderr(&output).contains(needle),
                "{args:?}: {}",
                stderr(&output)
            );
        }
        assert!(!sandbox.config().exists(), "input errors created settings");
        let output = sandbox.run(&[
            "--rail",
            net,
            "--wallet",
            &fixtures.invalid_address,
            "wallet",
            "show",
        ]);
        assert_eq!(code(&output), Some(2));
        mock.assert_untouched();
    });
}

#[test]
fn json_errors_are_one_object_on_stderr() {
    let sandbox = Sandbox::new("json");
    let output = sandbox.run(&["--json", "keys", "show", "missing"]);
    assert_eq!(code(&output), Some(3));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "config");
    assert_eq!(error["error"]["exit"], 3);
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing")
    );
    let output = sandbox.run(&["keys", "show", "missing"]);
    assert_eq!(
        stderr(&output),
        "error: no key \"missing\"; run allowit keys ls\n"
    );
}

#[test]
fn stdout_escapes_display_controls_and_redacts_the_engine_token() {
    let secret = "s".repeat(40);
    let mut sandbox = Sandbox::new("stdout-safe");
    sandbox
        .set("ALLOWIT_TOKEN", &secret)
        .set("ALLOWIT_URL", format!("https://x/\u{202e}{secret}"));
    let output = sandbox.run(&["config", "show"]);
    assert_eq!(code(&output), Some(0));
    let text = stdout(&output);
    assert!(!text.contains('\u{202e}'), "{text}");
    assert!(text.contains("\\u{202e}"), "{text}");
    assert!(!text.contains(&secret), "{text}");
    assert!(text.contains("[redacted]"), "{text}");
}

/// The rail's public profile fixture, seeded as an already verified export.
fn profile_sandbox(rail: &Rail<'_>) -> (Sandbox, Value) {
    let profile: Value = serde_json::from_slice(&rail.fixtures.public_profile).unwrap();
    let mut sandbox = Sandbox::new("public-profile");
    common::public_profile_fixture(&sandbox, &profile, "http://127.0.0.1:1");
    sandbox.set("ALLOWIT_URL", "not-a-server-origin");
    (sandbox, profile)
}

#[test]
fn public_profile_shows_without_agent_key_or_engine() {
    every_rail(|rail| {
        let net = rail.network;
        let (sandbox, profile) = profile_sandbox(rail);
        let shown = sandbox.json(&["--rail", net, "wallet", "show"]);
        assert_eq!(shown["network"], net);
        assert_eq!(shown["rail"], net);
        assert_eq!(shown["wallet"], profile["wallet"]);
        assert!(shown.get("signingSeed").is_none());
        assert!(shown.get("endpoints").is_none());
        assert!(shown.get("policy").is_none());
        let summary = sandbox.json(&["--rail", net, "policy", "show"]);
        assert_eq!(summary["policyId"], profile["policyId"]);
        let source = sandbox.ok(&["--rail", net, "policy", "show", "--source"]);
        assert!(!source.trim().is_empty());
        assert!(!source.contains('\x1b'));
        // With --json the source is one JSON object.
        let json = sandbox.json(&["--rail", net, "policy", "show", "--source"]);
        assert_eq!(
            json["source"].as_str().unwrap().trim_end(),
            source.trim_end()
        );
        assert_eq!(json.as_object().unwrap().len(), 1, "{json}");
        // Explicit selection keeps the same exact public profile binding.
        let wallet = profile["wallet"].as_str().unwrap();
        let output = sandbox.run(&["--rail", net, "--wallet", wallet, "wallet", "show"]);
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        assert!(stdout(&output).contains(wallet));
    });
}

#[test]
fn rail_conflict_and_secret_material_in_a_public_profile_fail_before_transport() {
    every_rail(|rail| {
        let net = rail.network;
        let (sandbox, mut profile) = profile_sandbox(rail);
        if let Some(other) = rail.other {
            let output = sandbox.run(&["--rail", other, "wallet", "show"]);
            assert_eq!(code(&output), Some(3));
            assert!(stderr(&output).contains("No verified public profile matches"));
            assert!(output.stdout.is_empty());
        }
        let other_wallet = rail.account(9).0;
        let output = sandbox.run(&["--rail", net, "--wallet", &other_wallet, "wallet", "show"]);
        assert_eq!(code(&output), Some(3), "{}", stderr(&output));
        assert!(stderr(&output).contains("No verified public profile matches"));
        profile["signingSeed"] = json!("must-remain-local");
        common::public_profile_fixture(&sandbox, &profile, "http://127.0.0.1:1");
        let output = sandbox.run(&["--rail", net, "wallet", "show"]);
        assert_eq!(code(&output), Some(3));
        assert!(stderr(&output).contains("must not contain signing material"));
        assert!(output.stdout.is_empty());
        assert!(!stdout(&output).contains("must-remain-local"));
    });
}

#[test]
fn wallet_selection_keeps_its_checksum_and_exact_profile_binding() {
    every_rail(|rail| {
        let net = rail.network;
        let (sandbox, profile) = profile_sandbox(rail);
        let wallet = profile["wallet"].as_str().unwrap();
        sandbox.ok(&["rail", "use", net]);
        sandbox.ok(&["wallet", "use", wallet]);
        assert_eq!(sandbox.json(&["wallet", "show"])["wallet"], wallet);
        assert_eq!(
            code(&sandbox.run(&["wallet", "use", &rail.fixtures.invalid_address])),
            Some(2)
        );
        assert_eq!(
            sandbox.json(&["config", "show"])["config"]["rails"][net]["wallet"],
            wallet
        );
        let other = rail.account(9).0;
        let refused = sandbox.run(&["--wallet", &other, "wallet", "show"]);
        assert_eq!(code(&refused), Some(3), "{}", stderr(&refused));
        assert!(stderr(&refused).contains("public profile"));
    });
}

#[cfg(unix)]
#[test]
fn arguments_that_are_not_utf8_are_usage_errors() {
    use std::os::unix::ffi::OsStrExt;
    let sandbox = Sandbox::new("utf8");
    let output = sandbox
        .command()
        .arg(std::ffi::OsStr::from_bytes(b"\xff"))
        .output()
        .unwrap();
    assert_eq!(code(&output), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains("not valid UTF-8"));
    let output = sandbox
        .command()
        .args([
            std::ffi::OsStr::new("--json"),
            std::ffi::OsStr::from_bytes(b"\xff"),
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), Some(2));
    let error: Value = serde_json::from_str(stderr(&output).trim()).unwrap();
    assert_eq!(error["error"]["code"], "usage");
}

#[test]
fn only_the_json_flag_selects_json_with_and_without_no_dna() {
    let mut sandbox = Sandbox::new("no-dna");
    for no_dna in [false, true] {
        if no_dna {
            sandbox.set("NO_DNA", "1");
        }
        // Piped output is text: the test harness reads stdout through a pipe.
        for args in [&["keys", "ls"][..], &["version"], &["rail", "ls"]] {
            let text = stdout(&sandbox.run(args));
            assert!(
                serde_json::from_str::<Value>(&text).is_err(),
                "{args:?}: {text}"
            );
            let mut json = vec!["--json"];
            json.extend_from_slice(args);
            let text = stdout(&sandbox.run(&json));
            serde_json::from_str::<Value>(&text).unwrap();
            // `NO_DNA` makes the JSON compact; it never selects it.
            // An empty key list is one line in both forms.
            if args[0] != "keys" {
                assert_eq!(text.lines().count() == 1, no_dna, "{args:?}: {text}");
            }
        }
        let output = sandbox.run(&["wallet", "show"]);
        assert_eq!(code(&output), Some(3));
        assert!(serde_json::from_str::<Value>(&stderr(&output)).is_err());
        let output = sandbox.run(&["--json", "exec", "--nope"]);
        assert_eq!(code(&output), Some(2));
        let error: Value = serde_json::from_str(&stderr(&output)).unwrap();
        assert_eq!(error["error"]["exit"], 2);
    }
    // The removed format flag is a usage error.
    assert_eq!(
        code(&sandbox.run(&["--output", "json", "version"])),
        Some(2)
    );
}

#[test]
fn verbose_details_are_data_lines() {
    every_rail(|rail| {
        let sandbox = Sandbox::new("verbose");
        sandbox.ok(&["rail", "use", rail.network]);
        let output = sandbox.run(&["-v", "rail", "show"]);
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        let text = stderr(&output);
        let line = format!("rail  {}", rail.network);
        assert!(text.lines().any(|l| l == line), "{text}");
        assert!(text.lines().any(|l| l.starts_with("config  ")), "{text}");
        common::assert_data(&text, "rail show -v");
    });
}
