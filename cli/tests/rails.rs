//! Rail selection, keys and the public wallet profiles: the rail decides the
//! key curve, the address formats and the adapter, and a command never reads
//! a credential, a profile or the network that it does not need.
mod common;

use common::{Mock, Sandbox, code, stderr, stdout};
use serde_json::{Value, json};

#[test]
fn rail_aliases_and_settings_stay_bound_to_the_selected_stellar_network() {
    let mut sandbox = Sandbox::new("rail");
    assert_eq!(code(&sandbox.run(&["keys", "generate", "agent"])), Some(3));
    assert_eq!(
        sandbox.json(&["rail", "use", "stellar:mainnet"])["rail"],
        "stellar:pubnet"
    );
    sandbox.ok(&["rail", "use", "stellar:testnet"]);
    let asset = allowit_stellar_wallet::asset_from_identity("XLM", None).unwrap();
    let contract = allowit_stellar_wallet::sac_address("stellar:testnet", &asset).unwrap();
    sandbox.ok(&["wallet", "use", &contract]);
    sandbox.ok(&["rail", "use", "stellar:pubnet"]);
    let config = sandbox.json(&["config", "show"]);
    assert!(config["config"]["rails"]["stellar:pubnet"]["wallet"].is_null());
    assert_eq!(
        config["config"]["rails"]["stellar:testnet"]["wallet"],
        contract
    );
    assert_eq!(config["railSource"], "config.toml");
    assert_eq!(
        code(&sandbox.run(&["rail", "use", "stellar:futurenet"])),
        Some(2)
    );
    sandbox.set("ALLOWIT_RAIL", "stellar:testnet");
    assert_eq!(sandbox.json(&["rail", "show"])["rail"], "stellar:testnet");
    assert_eq!(
        sandbox.json(&["--rail", "stellar:pubnet", "rail", "show"])["rail"],
        "stellar:pubnet"
    );
}

#[test]
fn stellar_keys_keep_private_material_out_of_public_metadata() {
    let sandbox = Sandbox::new("keys");
    sandbox.ok(&["rail", "use", "stellar:testnet"]);
    let stellar = sandbox.json(&["--rail", "stellar:testnet", "keys", "generate", "agent"]);
    assert_eq!(stellar["curve"], "ed25519");
    let address = stellar["address"].as_str().unwrap();
    // Stellar keys print as G strkeys.
    assert!(address.starts_with('G') && address.len() == 56);
    assert_eq!(stellar["publicKey"], stellar["address"]);
    // Private keys stay in secrets/, mode 0600 in a 0700 directory.
    let secret = sandbox.ok(&["keys", "export", "agent"]);
    let secret = secret.trim();
    assert!(secret.starts_with('S') && secret.len() == 56);
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
        sandbox.json(&["config", "show"])["config"]["rails"]["stellar:testnet"]["agent"],
        "agent"
    );
    assert_eq!(
        code(&sandbox.run(&["--rail", "stellar:pubnet", "keys", "use", "agent"])),
        Some(0)
    );
    // Re-import of the exported key under another name; the address is the same.
    let file = sandbox.private_file("agent.key", secret);
    let copy = sandbox.json(&[
        "--rail",
        "stellar:testnet",
        "keys",
        "import",
        "copy",
        file.to_str().unwrap(),
    ]);
    assert_eq!(copy["address"], stellar["address"]);
    // keys add records a public address that cannot sign.
    let owner = sandbox.json(&[
        "--rail",
        "stellar:testnet",
        "keys",
        "add",
        "owner",
        "--public-key",
        address,
    ]);
    assert_eq!(owner["secret"], false);
    assert_eq!(code(&sandbox.run(&["keys", "use", "owner"])), Some(3));
    let bad = sandbox.run(&[
        "--rail",
        "stellar:testnet",
        "keys",
        "add",
        "bad",
        "--public-key",
        "0x0000000000000000000000000000000000000000",
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
    let sandbox = Sandbox::new("concurrent");
    let addr = |n: u8| {
        let issuer = allowit_stellar_wallet::account_for_seed(&[n; 32]);
        let asset = allowit_stellar_wallet::asset_from_identity("USD", Some(&issuer)).unwrap();
        allowit_stellar_wallet::sac_address("stellar:testnet", &asset).unwrap()
    };
    let spawn_all = |args: &dyn Fn(&str) -> Vec<String>| {
        let children: Vec<_> = (1..=8u8)
            .map(|n| {
                let address = addr(n);
                let child = sandbox
                    .command()
                    .args(args(&address))
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                (address, child)
            })
            .collect();
        children
            .into_iter()
            .map(|(address, mut child)| (address, child.wait().unwrap().code()))
            .collect::<Vec<_>>()
    };
    // Each wallet use reads and rewrites config.toml; none loses another's wallet.
    for (_, exit) in spawn_all(&|a| {
        ["--rail", "stellar:testnet", "wallet", "use", a]
            .map(Into::into)
            .into()
    }) {
        assert_eq!(exit, Some(0));
    }
    let config = sandbox.json(&["config", "show"]);
    let mut wallets: Vec<_> = config["config"]["rails"]["stellar:testnet"]["wallets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().to_owned())
        .collect();
    wallets.sort();
    let mut expected = (1..=8).map(addr).collect::<Vec<_>>();
    expected.sort();
    assert_eq!(wallets, expected);
    // Only one keys add claims a name; the stored address is the winner's.
    let results = spawn_all(&|a| {
        [
            "--rail",
            "stellar:testnet",
            "keys",
            "add",
            "owner",
            "--public-key",
            &allowit_stellar_wallet::account_for_seed(&[a.as_bytes()[1]; 32]),
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
        allowit_stellar_wallet::account_for_seed(&[winners[0].0.as_bytes()[1]; 32])
    );
}

#[test]
fn faucet_funding_goes_through_the_rail_vendor() {
    let mut sandbox = Sandbox::new("fund");
    let mock = Mock::new();
    sandbox.set("ALLOWIT_FRIENDBOT_URL", &mock.url);
    sandbox.ok(&["rail", "use", "stellar:testnet"]);
    let key = sandbox.json(&["keys", "generate", "agent"]);
    let address = key["address"].as_str().unwrap().to_owned();
    let listener = mock.listener;
    let expected = address.clone();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let (head, _) = common::read_request(&mut conn);
        assert!(
            head.starts_with(&format!("GET /?addr={expected} ")),
            "{head}"
        );
        common::reply(&mut conn, 200, &json!({"hash": "ab".repeat(32)}));
    });
    let funded = sandbox.json(&["keys", "fund", "agent"]);
    assert_eq!(funded["address"], address);
    assert_eq!(funded["transactionHash"], "ab".repeat(32));
    server.join().unwrap();
    // Pubnet has no faucet.
    {
        let rail = "stellar:pubnet";
        let output = sandbox.run(&["--rail", rail, "keys", "fund", "agent"]);
        assert_eq!(code(&output), Some(3), "{rail}: {}", stderr(&output));
        // generate --fund refuses before the key exists, so a retry is clean.
        let output = sandbox.run(&["--rail", rail, "--json", "keys", "generate", "pk", "--fund"]);
        assert_eq!(code(&output), Some(3), "{rail}: {}", stderr(&output));
        assert!(output.stdout.is_empty(), "{rail}");
        assert_ne!(
            code(&sandbox.run(&["keys", "show", "pk"])),
            Some(0),
            "{rail}"
        );
    }
}

#[test]
fn invalid_stellar_input_fails_before_any_settings_bundle_or_network_access() {
    let mut sandbox = Sandbox::new("cross");
    let mock = Mock::new();
    sandbox
        .set("ALLOWIT_URL", &mock.url)
        .set("ALLOWIT_PROFILE_UNSUPPORTED", "/must-not-be-read");
    let invalid = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    let stellar = "GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5";
    for (args, needle) in [
        (
            vec![
                "--rail",
                "stellar:testnet",
                "exec",
                "--op",
                "transfer",
                "--to",
                invalid,
                "--amount",
                "XLM:1",
            ],
            "not a stellar recipient",
        ),
        (
            vec![
                "--rail",
                "stellar:testnet",
                "exec",
                "--op",
                "transfer",
                "--to",
                stellar,
                "--amount",
                "SOL:1",
            ],
            "SOL",
        ),
    ] {
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
        "stellar:testnet",
        "--wallet",
        "0x00",
        "wallet",
        "show",
    ]);
    assert_eq!(code(&output), Some(2));
    mock.assert_untouched();
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

fn stellar_approval_fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../engine/tests/fixtures/stellar-portable-approval.json"
    ))
    .unwrap()
}

fn public_stellar_profile(fixture: &Value) -> Value {
    use allowit_stellar_wallet::{self as sdk, WalletConfig, WalletRoute};
    let policy = &fixture["policy"];
    let mut config = WalletConfig {
        network: "stellar:testnet".into(),
        policy_instance: [7; 32],
        wallet: fixture["intent"]["owner"].as_str().unwrap().into(),
        owner: fixture["intent"]["owner"].as_str().unwrap().into(),
        agent: sdk::account_for_seed(&[2; 32]),
        server: sdk::account_for_seed(&[3; 32]),
        route: WalletRoute::Native,
        tier: None,
        language: sdk::PolicyLanguage::Portable,
        rules: vec![],
        params: serde_json::from_value(
            policy["runtime"]["instanceBinding"]["portable"]["params"].clone(),
        )
        .unwrap(),
        source_digest: [0; 32],
        asset: policy["mint"].as_str().unwrap().into(),
        asset_symbol: "XLM".into(),
        asset_issuer: None,
        policy_source: policy["code"].as_str().unwrap().into(),
        source_hash: sdk::unhex(policy["sourceHash"].as_str().unwrap()).unwrap(),
        module_digest: [5; 32],
        max_fee_stroops: 100,
    };
    let portable = common::portable_descriptor(&mut config);
    json!({"language":"allowit-portable-v1","portable":portable,"version":1,"profile":sdk::PROFILE,"network":config.network,
        "policyId":policy["id"],"sourceHash":policy["sourceHash"],"irHash":policy["irHash"],
        "moduleDigest":sdk::hex(&config.module_digest),"wallet":config.wallet,
        "route":"native","config":config,
        "endpoints":{"execute":"/api/native/execute","status":"/api/native/execute/status"}})
}

fn stellar_sandbox(profile: &Value) -> Sandbox {
    let mut sandbox = Sandbox::new("stellar-profile");
    let path = sandbox.work().join("public-stellar.json");
    std::fs::write(&path, serde_json::to_vec(profile).unwrap()).unwrap();
    common::public_profile_fixture(&sandbox, profile, "http://127.0.0.1:1");
    sandbox
        .set("ALLOWIT_STELLAR_SIGNER", "/must-never-be-opened")
        .set("ALLOWIT_URL", "not-a-server-origin");
    sandbox
}

#[test]
fn public_stellar_profile_shows_without_agent_key_or_engine() {
    let fixture = stellar_approval_fixture();
    let profile = public_stellar_profile(&fixture);
    let sandbox = stellar_sandbox(&profile);
    let shown = sandbox.json(&["--rail", "stellar:testnet", "wallet", "show"]);
    assert_eq!(shown["network"], "stellar:testnet");
    assert_eq!(shown["rail"], "stellar:testnet");
    assert_eq!(shown["agent"], profile["config"]["agent"]);
    assert_eq!(shown["wallet"], profile["wallet"]);
    assert!(shown.get("signingSeed").is_none());
    assert!(shown.get("endpoints").is_none());
    assert!(shown.get("policy").is_none());
    let summary = sandbox.json(&["--rail", "stellar:testnet", "policy", "show"]);
    assert_eq!(summary["policyId"], fixture["policy"]["id"]);
    assert_eq!(summary["asset"], "XLM");
    let source = sandbox.ok(&["--rail", "stellar:testnet", "policy", "show", "--source"]);
    assert_eq!(
        source.trim_end(),
        fixture["policy"]["code"].as_str().unwrap().trim_end()
    );
    assert!(!source.contains('\x1b'));
    // With --json the source is one JSON object.
    let source = sandbox.json(&["--rail", "stellar:testnet", "policy", "show", "--source"]);
    assert_eq!(
        source["source"].as_str().unwrap().trim_end(),
        fixture["policy"]["code"].as_str().unwrap().trim_end()
    );
    assert_eq!(source.as_object().unwrap().len(), 1, "{source}");
    // Explicit classic selection keeps the same exact public profile binding.
    let classic = profile["wallet"].as_str().unwrap();
    assert!(classic.starts_with('G'));
    let output = sandbox.run(&[
        "--rail",
        "stellar:testnet",
        "--wallet",
        classic,
        "wallet",
        "show",
    ]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).contains(classic));
}

#[test]
fn rail_conflict_and_secret_material_in_a_public_profile_fail_before_transport() {
    let fixture = stellar_approval_fixture();
    let mut profile = public_stellar_profile(&fixture);
    let sandbox = stellar_sandbox(&profile);
    let output = sandbox.run(&["--rail", "stellar:pubnet", "wallet", "show"]);
    assert_eq!(code(&output), Some(3));
    assert!(stderr(&output).contains("No verified public profile matches"));
    assert!(output.stdout.is_empty());
    let native = allowit_stellar_wallet::asset_from_identity("XLM", None).unwrap();
    let other = allowit_stellar_wallet::sac_address("stellar:testnet", &native).unwrap();
    let output = sandbox.run(&[
        "--rail",
        "stellar:testnet",
        "--wallet",
        &other,
        "wallet",
        "show",
    ]);
    assert_eq!(code(&output), Some(3), "{}", stderr(&output));
    assert!(stderr(&output).contains("No verified public profile matches"));
    profile["signingSeed"] = json!("must-remain-local");
    common::public_profile_fixture(&sandbox, &profile, "http://127.0.0.1:1");
    let output = sandbox.run(&["--rail", "stellar:testnet", "wallet", "show"]);
    assert_eq!(code(&output), Some(3));
    assert!(stderr(&output).contains("must not contain signing material"));
    assert!(output.stdout.is_empty());
    assert!(!stdout(&output).contains("must-remain-local"));
}

#[cfg(unix)]
#[test]
fn hidden_wallet_approve_exports_only_the_public_exact_intent() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let fixture = stellar_approval_fixture();
    let mut sandbox = Sandbox::new("approve");
    let file = sandbox.work().join("policy.json");
    let mut policy = fixture["policy"].clone();
    policy["signingSeed"] = json!("must-remain-local");
    std::fs::write(&file, serde_json::to_vec(&policy).unwrap()).unwrap();
    let owner = fixture["intent"]["owner"].as_str().unwrap();
    let request = fixture["intent"]["requestId"].as_str().unwrap();
    sandbox
        .set("ALLOWIT_URL", "https://app.allowit.xyz")
        .set("ALLOWIT_OWNER_KEYPAIR", "/must-never-be-opened")
        .set("ALLOWIT_STELLAR_SIGNER", "/must-never-be-opened");
    let output = sandbox.run(&[
        "--network",
        "stellar:testnet",
        "wallet",
        "approve",
        "--policy-file",
        file.to_str().unwrap(),
        "--owner",
        owner,
        "--request-id",
        request,
    ]);
    assert_eq!(code(&output), Some(10), "{}", stderr(&output));
    let shown: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(shown["state"], "awaiting_owner_approval");
    assert_eq!(shown["executed"], false);
    assert_eq!(shown["movesFunds"], false);
    let url = shown["signingUrl"].as_str().unwrap();
    let encoded = url.split_once("#allowit-stellar-approval=").unwrap().1;
    let public: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();
    assert_eq!(public, fixture["intent"]);
    assert!(!stdout(&output).contains("must-remain-local"));
    assert!(!stdout(&output).contains("must-never-be-opened"));
    // The link flow serves Stellar testnet only.
    let output = sandbox.run(&[
        "--network",
        "stellar:pubnet",
        "wallet",
        "approve",
        "--policy-file",
        file.to_str().unwrap(),
        "--owner",
        owner,
        "--request-id",
        request,
    ]);
    assert_eq!(code(&output), Some(3));
}

#[cfg(unix)]
#[test]
fn hidden_wallet_configure_keeps_an_expired_handoff_for_browser_recovery() {
    use allowit_stellar_wallet::{self as sdk, AccountState};
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut sandbox = Sandbox::new("configure");
    sandbox.set("ALLOWIT_URL", "https://app.allowit.xyz");
    let journal = sandbox.work().join(".allowit/stellar-owner-journal");
    std::fs::create_dir_all(&journal).unwrap();
    for path in [sandbox.work().join(".allowit"), journal.clone()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let state = AccountState {
        address: sdk::account_for_seed(&[1; 32]),
        sequence: 10,
        master_weight: 1,
        low_threshold: 0,
        medium_threshold: 0,
        high_threshold: 0,
        signers: vec![],
    };
    let agent = sdk::account_for_seed(&[2; 32]);
    let engine = sdk::account_for_seed(&[3; 32]);
    let id = "cli-expired-stellar-setup-001";
    let original =
        sdk::owner::prepare_native_setup(&state, &agent, &engine, id, "stellar:testnet", 100, 1)
            .unwrap();
    let path = journal.join(format!("request-{:x}.json", Sha256::digest(id.as_bytes())));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    serde_json::to_writer(file, &original).unwrap();
    let original_bytes = std::fs::read(&path).unwrap();
    let configure = |agent: &str| {
        sandbox.run(&[
            "--rail",
            "stellar:testnet",
            "wallet",
            "configure",
            "--owner",
            &state.address,
            "--agent",
            agent,
            "--engine",
            &engine,
            "--request-id",
            id,
        ])
    };
    let output = configure(&agent);
    assert_eq!(code(&output), Some(10), "{}", stderr(&output));
    let shown: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(shown["state"], "owner_intent_expired");
    assert_eq!(shown["recoveryOnly"], true);
    assert_eq!(shown["executed"], false);
    assert_eq!(shown["transactionHash"], original.transaction_hash);
    let encoded = shown["signingUrl"]
        .as_str()
        .unwrap()
        .split_once("#allowit-stellar-owner=")
        .unwrap()
        .1;
    let recovered: sdk::owner::NativeSetup =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();
    assert_eq!(recovered, original);
    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
    let conflict = configure(&sdk::account_for_seed(&[4; 32]));
    assert!(!conflict.status.success());
    assert_ne!(code(&conflict), Some(10));
    assert!(conflict.stdout.is_empty());
    assert!(stderr(&conflict).contains("already binds another intent"));
    assert_eq!(std::fs::read(&path).unwrap(), original_bytes);
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
fn no_dna_selects_compact_json_for_results_and_errors() {
    let mut sandbox = Sandbox::new("no-dna");
    sandbox.set("NO_DNA", "1");
    for args in [&["version"][..], &["rail", "ls"]] {
        let text = stdout(&sandbox.run(args));
        assert_eq!(text.lines().count(), 1, "{args:?}: {text}");
        serde_json::from_str::<Value>(&text).unwrap();
    }
    let output = sandbox.run(&["wallet", "show"]);
    assert_eq!(code(&output), Some(3));
    let error: Value = serde_json::from_str(&stderr(&output)).unwrap();
    assert_eq!(error["error"]["exit"], 3);
    let output = sandbox.run(&["exec", "--nope"]);
    assert_eq!(code(&output), Some(2));
    let error: Value = serde_json::from_str(&stderr(&output)).unwrap();
    assert_eq!(error["error"]["exit"], 2);
    // `--output text` still asks for text.
    let text = stdout(&sandbox.run(&["--output", "text", "version"]));
    assert!(serde_json::from_str::<Value>(&text).is_err(), "{text}");
}

#[test]
fn verbose_details_are_data_lines() {
    let sandbox = Sandbox::new("verbose");
    sandbox.ok(&["rail", "use", "stellar:testnet"]);
    let output = sandbox.run(&["-v", "rail", "show"]);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    let text = stderr(&output);
    assert!(text.lines().any(|l| l == "rail  stellar:testnet"), "{text}");
    assert!(text.lines().any(|l| l.starts_with("config  ")), "{text}");
    common::assert_data(&text, "rail show -v");
}

#[test]
fn classic_g_wallet_selection_keeps_checksum_and_exact_profile_binding() {
    let fixture = stellar_approval_fixture();
    let profile = public_stellar_profile(&fixture);
    let sandbox = stellar_sandbox(&profile);
    let wallet = profile["wallet"].as_str().unwrap();
    sandbox.ok(&["rail", "use", "stellar:testnet"]);
    sandbox.ok(&["wallet", "use", wallet]);
    assert_eq!(sandbox.json(&["wallet", "show"])["wallet"], wallet);
    let mut malformed = wallet.to_owned();
    malformed.pop();
    malformed.push(if wallet.ends_with('A') { 'B' } else { 'A' });
    assert_eq!(code(&sandbox.run(&["wallet", "use", &malformed])), Some(2));
    assert_eq!(
        sandbox.json(&["config", "show"])["config"]["rails"]["stellar:testnet"]["wallet"],
        wallet
    );
    let other = allowit_stellar_wallet::account_for_seed(&[9; 32]);
    let refused = sandbox.run(&["--wallet", &other, "wallet", "show"]);
    assert_eq!(code(&refused), Some(3), "{}", stderr(&refused));
    assert!(stderr(&refused).contains("public profile"));
}
