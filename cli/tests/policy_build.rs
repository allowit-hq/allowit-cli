//! `allowit policy build`: the build of a Custom policy's wallet code. The command compiles
//! nothing unless the server's source is the source that the owner signed. Every value comes
//! from the registered rail, so no test names a vendor.
mod common;

use allowit_sdk::{custom_build, plugins::Feature};
use common::{Mock, Rail, Sandbox, code, each_rail, stderr};
use serde_json::{Value, json};

const ID: &str = "Xg7IxRXCr2MhVdRrTFoYKeAw2iV4u463";
const TICKET: &str = "0123456789abcdef0123456789abcdef";

/// A public key of the rail, as the server key.
fn server_key(rail: &Rail<'_>, seed: u8) -> String {
    let keys = rail.plugin.keys();
    keys.derive(&vec![seed; keys.entropy_bytes()])
        .unwrap()
        .address()
        .to_owned()
}

/// The policy source of the CLI template: it passes the policy source gate.
fn source() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/policy/src/policy.rs"),
    )
    .unwrap()
}

/// The server's build reply for `source`.
fn admission(rail: &Rail<'_>, source: &str) -> Value {
    json!({
        "policyId": ID,
        "network": rail.network,
        "source": source,
        "build": null,
    })
}

/// Runs `args` against a server that answers `replies` in order, and returns the output with
/// the request line of each request that arrived.
fn served(
    sandbox: &mut Sandbox,
    args: &[&str],
    replies: Vec<(u16, Value)>,
) -> (std::process::Output, Vec<String>) {
    served_by(Mock::new(), sandbox, args, replies)
}

/// The same, on a server that the test created before, so that a setting can name its origin.
fn served_by(
    mock: Mock,
    sandbox: &mut Sandbox,
    args: &[&str],
    replies: Vec<(u16, Value)>,
) -> (std::process::Output, Vec<String>) {
    sandbox.set("ALLOWIT_URL", &mock.url);
    let url = mock.url.clone();
    let listener = mock.listener;
    let expected = replies.len();
    let server = std::thread::spawn(move || {
        let mut seen = vec![];
        for (status, body) in replies {
            let (mut conn, _) = listener.accept().unwrap();
            // The wake-up connection of a command that sent fewer requests carries no request.
            let Ok((head, _)) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                common::read_request(&mut conn)
            })) else {
                break;
            };
            common::reply(&mut conn, status, &body);
            seen.push(head.lines().next().unwrap_or_default().to_owned());
        }
        seen
    });
    let output = sandbox.run(args);
    let _ = std::net::TcpStream::connect(url.trim_start_matches("http://"));
    let seen = server.join().unwrap_or_default();
    assert!(seen.len() <= expected);
    (output, seen)
}

fn error(output: &std::process::Output) -> Value {
    serde_json::from_str(stderr(output).lines().last().unwrap_or_default())
        .unwrap_or_else(|_| panic!("no JSON error: {}", stderr(output)))
}

#[test]
fn a_build_needs_the_ticket_and_signed_source_hash_or_an_owner_key() {
    let mut sandbox = Sandbox::new("build-usage");
    let mock = Mock::new();
    sandbox.set("ALLOWIT_URL", &mock.url);
    let hash = allowit_sdk::digest(source().as_bytes());
    let output = sandbox.run(&["policy", "build", ID]);
    assert_eq!(code(&output), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains("--ticket"), "{}", stderr(&output));
    // A ticket needs the signed source hash with it.
    let output = sandbox.run(&["policy", "build", ID, "--ticket", TICKET]);
    assert_eq!(code(&output), Some(2), "{}", stderr(&output));
    let output = sandbox.run(&[
        "policy",
        "build",
        ID,
        "--ticket",
        TICKET,
        "--source-hash",
        "not a hash",
    ]);
    assert_eq!(code(&output), Some(2), "{}", stderr(&output));
    let output = sandbox.run(&[
        "policy",
        "build",
        "not a policy id",
        "--ticket",
        TICKET,
        "--source-hash",
        &hash,
    ]);
    assert_eq!(code(&output), Some(2), "{}", stderr(&output));
    mock.assert_untouched();
}

#[test]
fn source_that_the_owner_did_not_sign_stops_the_build_before_it_compiles() {
    each_rail(Feature::CustomBuild, |rail| {
        let mut sandbox = Sandbox::new("build-source");
        let signed = allowit_sdk::digest(b"pub fn execute() { /* the signed source */ }\n");
        let (output, requests) = served(
            &mut sandbox,
            &[
                "--json",
                "policy",
                "build",
                ID,
                "--ticket",
                TICKET,
                "--source-hash",
                &signed,
            ],
            vec![(200, admission(rail, &source())), (200, json!({}))],
        );
        assert_eq!(code(&output), Some(3), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("differs from the source that the owner signed"),
            "{}",
            stderr(&output)
        );
        // The source was read; nothing was built or handed over.
        assert_eq!(requests.len(), 1, "{requests:?}");
        assert!(
            requests[0].starts_with("GET /api/custom/build?"),
            "{requests:?}"
        );
        assert!(!sandbox.config().join("custom-build").exists());
    });
}

#[test]
fn a_missing_build_tool_is_a_typed_refusal_that_says_what_to_install() {
    each_rail(Feature::CustomBuild, |rail| {
        let source = source();
        let mut sandbox = Sandbox::new("build-toolchain");
        // No build tool is on this PATH.
        let empty = sandbox.work().join("no-tools");
        std::fs::create_dir_all(&empty).unwrap();
        sandbox.set("PATH", &empty);
        let hash = allowit_sdk::digest(source.as_bytes());
        let (output, requests) = served(
            &mut sandbox,
            &[
                "--json",
                "policy",
                "build",
                ID,
                "--ticket",
                TICKET,
                "--source-hash",
                &hash,
            ],
            vec![(200, admission(rail, &source)), (200, json!({}))],
        );
        assert_eq!(code(&output), Some(2), "{}", stderr(&output));
        let error = error(&output);
        assert_eq!(
            error["error"]["api_code"],
            custom_build::code::TOOLCHAIN_MISSING,
            "{error}"
        );
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Install"),
            "{error}"
        );
        // The source was read; no code was handed over.
        assert_eq!(requests.len(), 1);
    });
}

#[test]
fn the_server_key_of_the_command_must_equal_the_pinned_key() {
    each_rail(Feature::CustomBuild, |rail| {
        let (pinned, other) = (server_key(rail, 1), server_key(rail, 3));
        let mut sandbox = Sandbox::new("build-pinned");
        let mock = Mock::new();
        sandbox.set("ALLOWIT_URL", &mock.url);
        sandbox.ok(&["--rail", rail.network, "rail", "use", rail.network]);
        sandbox.ok(&["--rail", rail.network, "rail", "trust", &mock.url, &pinned]);
        sandbox.ok(&["keys", "generate", "owner"]);
        // The command names another key than the one that `rail trust` pinned for this server.
        let (output, requests) = served_by(
            mock,
            &mut sandbox,
            &[
                "policy",
                "build",
                ID,
                "--owner-key",
                "owner",
                "--origin",
                "http://localhost:5173",
                "--server-key",
                &other,
            ],
            vec![(503, json!({"error": "unavailable"}))],
        );
        assert_eq!(code(&output), Some(3), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("--server-key differs"),
            "{}",
            stderr(&output)
        );
        assert!(requests.is_empty(), "{requests:?}");
        assert!(!sandbox.config().join("custom-build").exists());
    });
}

#[test]
fn an_owner_key_with_the_server_key_of_the_command_needs_no_pinned_key() {
    each_rail(Feature::CustomBuild, |rail| {
        let server = server_key(rail, 1);
        let mut sandbox = Sandbox::new("build-owner-key");
        sandbox.ok(&["--rail", rail.network, "rail", "use", rail.network]);
        sandbox.ok(&["keys", "generate", "owner"]);
        // No `rail trust` ran. The command names the server key, so the owner signs in with it.
        let (output, requests) = served(
            &mut sandbox,
            &[
                "policy",
                "build",
                ID,
                "--owner-key",
                "owner",
                "--origin",
                "http://localhost:5173",
                "--server-key",
                &server,
            ],
            vec![(503, json!({"error": "unavailable"}))],
        );
        assert_ne!(code(&output), Some(0));
        assert!(
            !stderr(&output).contains("no trusted server key"),
            "{}",
            stderr(&output)
        );
        assert_eq!(requests.len(), 1, "{}", stderr(&output));
        assert!(requests[0].starts_with("POST /api/auth/"), "{requests:?}");
    });
}
