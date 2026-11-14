//! `allowit policy build`: the build of a Custom policy's wallet code. The command compiles
//! nothing before the trusted server key signed the exact source that it received. Every value
//! comes from the registered rail, so no test names a vendor.
mod common;

use allowit_sdk::{custom_build, plugins::Feature};
use common::{Mock, Rail, Sandbox, code, each_rail, stderr};
use serde_json::{Value, json};

const ID: &str = "Xg7IxRXCr2MhVdRrTFoYKeAw2iV4u463";
const TICKET: &str = "0123456789abcdef0123456789abcdef";

/// A local key of the rail as the signer of a build statement.
struct Key {
    network: &'static str,
    key: Box<dyn allowit_sdk::portable::vendor::PrivateSigner>,
}
impl Key {
    fn of(rail: &Rail<'_>, seed: u8) -> Self {
        let keys = rail.plugin.keys();
        Self {
            network: rail.network,
            key: keys.derive(&vec![seed; keys.entropy_bytes()]).unwrap(),
        }
    }
    fn address(&self) -> &str {
        self.key.address()
    }
}
impl custom_build::Signer for Key {
    fn network(&self) -> &str {
        self.network
    }
    fn address(&self) -> &str {
        self.key.address()
    }
    fn sign_hash(&self, digest: &[u8]) -> allowit_sdk::error::Result<Vec<u8>> {
        self.key.sign_hash(digest)
    }
}

/// The policy source of the CLI template: it passes the policy source gate.
fn source() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/policy/src/policy.rs"),
    )
    .unwrap()
}

/// The server's build reply for `source`, presigned by `server` for `signed`.
fn admission(rail: &Rail<'_>, server: &Key, signed: &str, source: &str) -> Value {
    json!({
        "policyId": ID,
        "network": rail.network,
        "owner": Key::of(rail, 2).address(),
        "source": source,
        "presignature": custom_build::presign(server, ID, signed).unwrap(),
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
fn a_build_needs_a_ticket_or_an_owner_key_and_reaches_no_server_without_one() {
    let sandbox = Sandbox::new("build-usage");
    let mock = Mock::new();
    let mut sandbox = sandbox;
    sandbox.set("ALLOWIT_URL", &mock.url);
    let output = sandbox.run(&["policy", "build", ID]);
    assert_eq!(code(&output), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains("--ticket"), "{}", stderr(&output));
    let output = sandbox.run(&["policy", "build", "not a policy id", "--ticket", TICKET]);
    assert_eq!(code(&output), Some(2), "{}", stderr(&output));
    mock.assert_untouched();
}

#[test]
fn a_presignature_of_an_untrusted_server_key_stops_the_build_before_it_compiles() {
    each_rail(Feature::CustomBuild, |rail| {
        let (trusted, untrusted) = (Key::of(rail, 1), Key::of(rail, 3));
        let source = source();
        let mut sandbox = Sandbox::new("build-untrusted");
        // The server signs with another key than the one of the owner's build command.
        let (output, requests) = served(
            &mut sandbox,
            &[
                "--json",
                "policy",
                "build",
                ID,
                "--ticket",
                TICKET,
                "--server-key",
                trusted.address(),
            ],
            vec![
                (200, admission(rail, &untrusted, &source, &source)),
                (200, json!({})),
            ],
        );
        assert_eq!(code(&output), Some(2), "{}", stderr(&output));
        assert_eq!(
            error(&output)["error"]["api_code"],
            custom_build::code::PRESIGNATURE_MISMATCH
        );
        // Only the read arrived: no code was built or handed over.
        assert_eq!(
            requests,
            [format!(
                "GET /api/custom/build?id={ID}&ticket={TICKET} HTTP/1.1"
            )]
        );
        assert!(!sandbox.config().join("custom-build").exists());
    });
}

#[test]
fn source_that_the_server_did_not_presign_stops_the_build_before_it_compiles() {
    each_rail(Feature::CustomBuild, |rail| {
        let server = Key::of(rail, 1);
        let source = source();
        let altered = format!("{source}// altered\n");
        let mut sandbox = Sandbox::new("build-altered");
        let (output, requests) = served(
            &mut sandbox,
            &[
                "--json",
                "policy",
                "build",
                ID,
                "--ticket",
                TICKET,
                "--server-key",
                server.address(),
            ],
            vec![
                (200, admission(rail, &server, &source, &altered)),
                (200, json!({})),
            ],
        );
        assert_eq!(code(&output), Some(2), "{}", stderr(&output));
        assert_eq!(
            error(&output)["error"]["api_code"],
            custom_build::code::PRESIGNATURE_MISMATCH
        );
        assert_eq!(requests.len(), 1);
        assert!(!sandbox.config().join("custom-build").exists());
    });
}

#[test]
fn the_server_key_of_the_command_must_equal_the_pinned_key() {
    each_rail(Feature::CustomBuild, |rail| {
        let (pinned, other) = (Key::of(rail, 1), Key::of(rail, 3));
        let source = source();
        let mut sandbox = Sandbox::new("build-pinned");
        let mock = Mock::new();
        sandbox.set("ALLOWIT_URL", &mock.url);
        sandbox.ok(&[
            "--rail",
            rail.network,
            "rail",
            "trust",
            &mock.url,
            pinned.address(),
        ]);
        // The command names another key than the one that `rail trust` pinned for this server.
        let (output, _) = served_by(
            mock,
            &mut sandbox,
            &[
                "policy",
                "build",
                ID,
                "--ticket",
                TICKET,
                "--server-key",
                other.address(),
            ],
            vec![(200, admission(rail, &other, &source, &source))],
        );
        assert_eq!(code(&output), Some(3), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("--server-key differs"),
            "{}",
            stderr(&output)
        );
        assert!(!sandbox.config().join("custom-build").exists());
    });
}

#[test]
fn a_missing_build_tool_is_a_typed_refusal_that_says_what_to_install() {
    each_rail(Feature::CustomBuild, |rail| {
        let server = Key::of(rail, 1);
        let source = source();
        let mut sandbox = Sandbox::new("build-toolchain");
        // No build tool is on this PATH.
        let empty = sandbox.work().join("no-tools");
        std::fs::create_dir_all(&empty).unwrap();
        sandbox.set("PATH", &empty);
        let (output, requests) = served(
            &mut sandbox,
            &[
                "--json",
                "policy",
                "build",
                ID,
                "--ticket",
                TICKET,
                "--server-key",
                server.address(),
            ],
            vec![
                (200, admission(rail, &server, &source, &source)),
                (200, json!({})),
            ],
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
