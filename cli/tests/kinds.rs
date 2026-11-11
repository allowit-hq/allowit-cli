//! Wallet kinds, server selection and typed server refusals. The four kinds,
//! their text and what each rail supports come from the SDK; a test compares
//! the CLI with the SDK and never names a vendor.
mod common;

use allowit_sdk::kinds::{WalletKind, wallet_kind, wallet_kinds};
use common::{Mock, Sandbox, code, every_rail, stderr, stdout};
use serde_json::{Value, json};

const PROMPT: &str = "Pay only my approved recipient";

fn generate<'a>(net: &'a str, kind: &'a str) -> [&'a str; 12] {
    [
        "--rail",
        net,
        "policy",
        "generate",
        "--owner-key",
        "owner",
        "--origin",
        "http://localhost:5173",
        "--kind",
        kind,
        "--new-wallet",
        PROMPT,
    ]
}

#[test]
fn policy_kinds_lists_the_kinds_of_the_rail_as_the_sdk_describes_them() {
    every_rail(|rail| {
        let net = rail.network;
        let sandbox = Sandbox::new("kinds");
        let kinds = wallet_kinds(net).unwrap();
        assert!(!kinds.is_empty(), "{net} supports no wallet kind");
        let listed = sandbox.json(&["--rail", net, "policy", "kinds"]);
        assert_eq!(listed, serde_json::to_value(&kinds).unwrap());
        let text = sandbox.ok(&["--rail", net, "policy", "kinds"]);
        common::assert_data(&text, "policy kinds");
        assert_eq!(text.split("\n\n").count(), kinds.len());
        for (block, info) in text.split("\n\n").zip(&kinds) {
            let value = |key: &str| {
                block
                    .lines()
                    .find_map(|line| line.strip_prefix(key))
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_owned()
            };
            assert_eq!(value("id "), info.id.id());
            assert_eq!(value("title "), info.title);
            assert_eq!(value("explanation "), info.explanation);
            let json = serde_json::to_value(info).unwrap();
            assert_eq!(value("enforcement "), json["enforcement"]);
            assert_eq!(value("custody "), json["custody"]);
            // Only a kind with a fixed list shows its params and their link.
            assert_eq!(value("supportedParams "), info.supported_params.join(", "));
            assert_eq!(value("paramsUrl "), info.params_url.unwrap_or_default());
        }
        assert!(!sandbox.config().exists(), "a listing created settings");
    });
}

#[test]
fn generate_help_explains_every_kind_with_the_sdk_text() {
    let sandbox = Sandbox::new("kind-help");
    let help = sandbox.ok(&["policy", "generate", "--help"]);
    assert!(help.contains("--kind <KIND>"));
    assert!(help.contains("native, simple, standard, custom"));
    for kind in WalletKind::ALL {
        let info = kind.info();
        let line = format!("{}: {}", info.title, info.explanation);
        assert!(
            help.lines().any(|l| l.trim_end().ends_with(&line)),
            "{line}"
        );
        if let Some(url) = info.params_url {
            assert!(help.contains(&format!("Supported params: {url}")));
        }
    }
    assert!(!help.contains("common"), "{help}");
}

#[test]
fn a_retired_or_unknown_kind_is_a_usage_error_before_any_access() {
    every_rail(|rail| {
        let net = rail.network;
        let mut sandbox = Sandbox::new("kind-guard");
        let mock = Mock::new();
        sandbox.set("ALLOWIT_URL", &mock.url);
        for kind in ["common", "Standard", "policy-wallet"] {
            let output = sandbox.run(&generate(net, kind));
            assert_eq!(code(&output), Some(2), "{kind}: {}", stderr(&output));
            assert!(
                stderr(&output).contains("native, simple, standard, custom"),
                "{}",
                stderr(&output)
            );
            assert!(output.stdout.is_empty());
        }
        // A kind that the rail does not support exits 3 and names what it does.
        let supported = wallet_kinds(net).unwrap();
        for kind in WalletKind::ALL {
            if supported.iter().any(|info| info.id == kind) {
                continue;
            }
            let output = sandbox.run(&generate(net, kind.id()));
            assert_eq!(code(&output), Some(3), "{}", stderr(&output));
            assert!(stderr(&output).contains("is not supported on"));
        }
        assert!(!sandbox.config().exists(), "input errors created settings");
        mock.assert_untouched();
    });
}

#[test]
fn a_missing_server_url_and_an_untrusted_server_have_their_own_errors() {
    every_rail(|rail| {
        let net = rail.network;
        let mut sandbox = Sandbox::new("server");
        // The sandbox presets a server URL; an empty value is an unset one.
        sandbox.set("ALLOWIT_URL", "");
        sandbox.ok(&["rail", "use", net]);
        sandbox.ok(&["keys", "generate", "owner"]);
        // No ALLOWIT_URL: the error names the variable, not server trust.
        for kind in wallet_kinds(net).unwrap() {
            let output = sandbox.run(&generate(net, kind.id.id()));
            assert_eq!(code(&output), Some(3), "{}", stderr(&output));
            assert_eq!(
                stderr(&output),
                "error: ALLOWIT_URL is not set; set it to the URL of the Allowit server\n"
            );
        }
        let profile: Value = serde_json::from_slice(&rail.fixtures.public_profile).unwrap();
        common::public_profile_fixture(&sandbox, &profile, "http://127.0.0.1:1");
        let status = ["--rail", net, "status", "pay-0001"];
        let output = sandbox.run(&status);
        assert_eq!(code(&output), Some(3), "{}", stderr(&output));
        assert!(
            stderr(&output).starts_with("error: ALLOWIT_URL is not set"),
            "{}",
            stderr(&output)
        );
        // A URL without a pinned identity: the error names the origin and the command.
        let mock = Mock::new();
        sandbox.set("ALLOWIT_URL", &mock.url);
        let output = sandbox.run(&status);
        assert_eq!(code(&output), Some(3), "{}", stderr(&output));
        assert_eq!(
            stderr(&output),
            format!(
                "error: no trusted server key for {url} on {net}; run allowit rail trust {url} PUBLIC_KEY\n",
                url = mock.url
            )
        );
        mock.assert_untouched();
    });
}

#[test]
fn a_typed_server_refusal_prints_the_server_code_and_reason() {
    every_rail(|rail| {
        let net = rail.network;
        let Some(kind) = wallet_kinds(net).unwrap().first().map(|info| info.id) else {
            return;
        };
        let reason = format!("Wallet kind {kind} is not enabled on {net}");
        for json_mode in [false, true] {
            let mut sandbox = Sandbox::new("refusal");
            let mock = Mock::new();
            sandbox.set("ALLOWIT_URL", &mock.url);
            sandbox.ok(&["rail", "use", net]);
            sandbox.ok(&["keys", "generate", "owner"]);
            let listener = mock.listener;
            let body = json!({
                "error": reason,
                "code": "wallet_kind_unsupported",
                "details": {"walletKind": kind.id(), "network": net, "reason": reason},
            });
            // The stub refuses every request of the command with the typed refusal.
            let server = std::thread::spawn(move || {
                listener.set_nonblocking(false).unwrap();
                let (mut conn, _) = listener.accept().unwrap();
                let _ = common::read_request(&mut conn);
                common::reply(&mut conn, 400, &body);
            });
            let mut args = generate(net, kind.id()).to_vec();
            if json_mode {
                args.insert(0, "--json");
            }
            let output = sandbox.run(&args);
            server.join().unwrap();
            assert!(output.stdout.is_empty(), "{}", stdout(&output));
            assert_eq!(code(&output), Some(4), "{}", stderr(&output));
            if json_mode {
                let error: Value = serde_json::from_slice(&output.stderr).unwrap();
                assert_eq!(error["error"]["api_code"], "wallet_kind_unsupported");
                assert_eq!(error["error"]["code"], "rejected");
                assert!(
                    error["error"]["message"]
                        .as_str()
                        .unwrap()
                        .contains(&reason)
                );
            } else {
                let text = stderr(&output);
                assert!(
                    text.starts_with("error: wallet_kind_unsupported: "),
                    "{text}"
                );
                assert!(text.contains(&reason), "{text}");
            }
        }
    });
}

#[test]
fn wallet_show_reports_the_kind_that_the_profile_binds() {
    every_rail(|rail| {
        let net = rail.network;
        let profile: Value = serde_json::from_slice(&rail.fixtures.public_profile).unwrap();
        let sandbox = Sandbox::new("wallet-kind");
        common::public_profile_fixture(&sandbox, &profile, "http://127.0.0.1:1");
        let expected = wallet_kind(net, &rail.fixtures.public_profile)
            .ok()
            .map(WalletKind::id);
        for command in ["wallet", "policy"] {
            let shown = sandbox.json(&["--rail", net, command, "show"]);
            assert_eq!(shown.get("walletKind").and_then(Value::as_str), expected);
            let text = sandbox.ok(&["--rail", net, command, "show"]);
            common::assert_data(&text, "show");
            assert_eq!(
                text.lines().any(|line| line.starts_with("walletKind ")),
                expected.is_some()
            );
            assert!(!text.contains("UNIT"), "{text}");
        }
    });
}

/// Runs `args` against a stub server that answers one request with `status`
/// and `body`, and returns the output and the request line of that request.
fn against_stub(
    net: &str,
    args: &[&str],
    status: u16,
    body: Value,
) -> (std::process::Output, String) {
    let mut sandbox = Sandbox::new("kinds-server");
    let mock = Mock::new();
    sandbox.set("ALLOWIT_URL", &mock.url);
    sandbox.ok(&["rail", "use", net]);
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let (head, _) = common::read_request(&mut conn);
        common::reply(&mut conn, status, &body);
        head.lines().next().unwrap_or_default().to_owned()
    });
    let output = sandbox.run(args);
    (output, server.join().unwrap())
}

#[test]
fn policy_kinds_server_lists_what_the_server_accepts_on_the_rail() {
    every_rail(|rail| {
        let net = rail.network;
        // The server accepts a subset of the kinds, in its own order.
        let offered: Vec<Value> = wallet_kinds(net)
            .unwrap()
            .into_iter()
            .rev()
            .skip(1)
            .map(|info| serde_json::to_value(info).unwrap())
            .collect();
        let body = json!({"network": net, "kinds": offered});
        let (output, request) = against_stub(
            net,
            &["--json", "policy", "kinds", "--server"],
            200,
            body.clone(),
        );
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        assert_eq!(
            request,
            format!("GET /api/wallet-kinds?network={net} HTTP/1.1")
        );
        let listed: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(listed, Value::Array(offered.clone()));
        // With --json, stderr carries no progress line.
        assert!(output.stderr.is_empty(), "{}", stderr(&output));
        // Text prints the same kinds as data and the progress step on stderr.
        let (output, _) = against_stub(net, &["policy", "kinds", "--server"], 200, body);
        assert_eq!(code(&output), Some(0), "{}", stderr(&output));
        let text = stdout(&output);
        common::assert_data(&text, "policy kinds --server");
        for kind in &offered {
            assert!(text.contains(kind["explanation"].as_str().unwrap()));
        }
        assert_eq!(
            stderr(&output),
            "Reading the wallet kinds from the Allowit server\n"
        );
        // -q drops the progress line.
        let body = json!({"network": net, "kinds": offered});
        let (output, _) = against_stub(net, &["-q", "policy", "kinds", "--server"], 200, body);
        assert!(output.stderr.is_empty(), "{}", stderr(&output));
    });
}

#[test]
fn policy_kinds_server_refuses_an_invalid_list_and_prints_typed_refusals() {
    every_rail(|rail| {
        let net = rail.network;
        let args = ["policy", "kinds", "--server"];
        let kind = serde_json::to_value(WalletKind::Native.info()).unwrap();
        let mut retired = kind.clone();
        retired["id"] = "common".into();
        for (body, needle) in [
            (
                json!({"network": net, "kinds": [retired]}),
                "invalid wallet-kind list",
            ),
            (
                json!({"network": "other:net", "kinds": []}),
                "another network",
            ),
            (
                json!({"network": net, "kinds": [kind, kind]}),
                "more than once",
            ),
            (json!({"kinds": []}), "invalid wallet-kind list"),
        ] {
            let (output, _) = against_stub(net, &args, 200, body);
            assert_eq!(code(&output), Some(5), "{}", stderr(&output));
            assert!(output.stdout.is_empty());
            assert!(stderr(&output).contains(needle), "{}", stderr(&output));
        }
        let refusal = json!({"error": "This network is not served", "code": "network_unsupported"});
        let (output, _) = against_stub(net, &args, 400, refusal);
        assert_eq!(code(&output), Some(4), "{}", stderr(&output));
        assert!(
            stderr(&output).ends_with(
                "error: network_unsupported: Allowit returned HTTP 400: This network is not served\n"
            ),
            "{}",
            stderr(&output)
        );
        let down = json!({"error": "Network unavailable", "code": "network_unavailable"});
        let (output, _) = against_stub(net, &args, 503, down);
        assert_eq!(code(&output), Some(7), "{}", stderr(&output));
    });
}

#[test]
fn wallet_show_prints_each_param_in_human_units_with_the_asset_unit() {
    every_rail(|rail| {
        let net = rail.network;
        let profile: Value = serde_json::from_slice(&rail.fixtures.public_profile).unwrap();
        let sandbox = Sandbox::new("wallet-params");
        common::public_profile_fixture(&sandbox, &profile, "http://127.0.0.1:1");
        for command in ["wallet", "policy"] {
            let shown = sandbox.json(&["--rail", net, command, "show"]);
            let text = sandbox.ok(&["--rail", net, command, "show"]);
            let stored = shown["params"].as_object().unwrap();
            let Some(display) = shown["paramsDisplay"].as_array() else {
                // A rail that does not know the asset prints the stored values.
                continue;
            };
            // JSON keeps the stored values and adds one display entry per param.
            assert_eq!(display.len(), stored.len());
            for field in display {
                let key = field["key"].as_str().unwrap();
                let value = field["shown"].as_str().unwrap();
                assert!(stored.contains_key(key), "{key}");
                assert!(!field["label"].as_str().unwrap().is_empty());
                // An amount carries the unit of the installed asset, never a placeholder.
                if let Some(unit) = field["unit"].as_str() {
                    assert_eq!(unit, shown["assetSymbol"]);
                    assert!(value.ends_with(&format!(" {unit}")), "{value}");
                    assert_ne!(unit, "UNIT");
                }
                // Text prints the display value in place of the stored one.
                let row = text
                    .lines()
                    .find_map(|line| line.strip_prefix(&format!("params.{key} ")))
                    .unwrap_or_else(|| panic!("no params.{key} row in\n{text}"));
                assert_eq!(row.trim(), value);
                assert!(!text.contains(&format!("params.{key}.")), "{text}");
            }
        }
    });
}
