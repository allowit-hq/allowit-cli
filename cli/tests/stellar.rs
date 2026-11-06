//! Stellar agent requests against a mock engine: `exec --op transfer` and
//! `status ID` with the agent key from `keys use` and the public wallet
//! profile. The mock checks the agent's request proof and real signatures.
#![cfg(unix)]
#[path = "../../engine/tests/support/owner_chain.rs"]
mod chain_fixture;
mod common;
#[path = "../../engine/tests/support/owner_server.rs"]
mod wire;

use allowit_stellar_wallet::workflow::*;
use allowit_stellar_wallet::{self as sdk, *};
use common::{Mock, Sandbox, code, read_request, reply, stderr};
use serde_json::{Value, json};
use std::{
    net::TcpStream,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use stellar_xdr::{
    Limits, OperationResult, OperationResultTr, PaymentResult, TransactionResult,
    TransactionResultExt, TransactionResultResult, WriteXdr,
};

const POLICY: &str = "stellar-policy";
const ID: &str = "stellar-run-0001";

fn nonce() -> [u8; 32] {
    let mut v = b"allowit-workflow-operation-v1\0".to_vec();
    v.extend(POLICY.as_bytes());
    v.push(0);
    v.extend(ID.as_bytes());
    sdk::hash(v)
}
fn acct(s: &str) -> AccountId {
    AccountId {
        chain: Chain::Stellar,
        kind: AccountKind::Wallet,
        address: sdk::account_key(s).unwrap(),
    }
}
fn dest() -> String {
    sdk::account_for_seed(&[6; 32])
}

/// The wallet, the request that `exec` must build from its flags, the chain
/// state and the current time.
fn fixture() -> (WalletConfig, ExecutionRequest, TrustedState, u64) {
    let owner = sdk::account_for_seed(&[1; 32]);
    let agent = sdk::account_for_seed(&[2; 32]);
    let engine = sdk::account_for_seed(&[3; 32]);
    let source = "use allowit::prelude::*; pub fn execute(ctx: &AllowitContext) -> allowit::Result { Ok(()) }".to_string();
    let mut c = WalletConfig {
        network: "stellar:testnet".into(),
        policy_instance: [7; 32],
        wallet: owner.clone(),
        owner: owner.clone(),
        agent: agent.clone(),
        server: engine.clone(),
        route: WalletRoute::Native,
        tier: None,
        language: sdk::PolicyLanguage::Portable,
        rules: vec![],
        params: Default::default(),
        source_digest: [0; 32],
        asset: String::new(),
        asset_symbol: "XLM".into(),
        asset_issuer: None,
        source_hash: sdk::hash(&source),
        policy_source: source,
        module_digest: [5; 32],
        max_fee_stroops: 50000,
    };
    c.asset = sdk::sac_address(&c.network, &sdk::asset_xdr(&c).unwrap()).unwrap();
    common::portable_descriptor(&mut c);
    let asset = WorkflowAsset::Native {
        chain: Chain::Stellar,
        network: sdk::network_id(&c.network).unwrap(),
    };
    let r = ExecutionRequest {
        operation: OperationRef {
            policy_instance: c.policy_instance,
            nonce: nonce(),
        },
        input: ExecutionInput::Instructions(vec![NativeInstruction::StellarNativeTransfer {
            source: acct(&owner),
            destination: acct(&dest()),
            stroops: 1_000_000,
        }]),
        effect_bounds: vec![WorkflowEffectBound {
            asset: asset.clone(),
            source: acct(&owner),
            beneficiary: acct(&dest()),
            max_debit: 1_000_000u64.into(),
            min_credit: 1_000_000u64.into(),
            max_burn: 0u64.into(),
        }],
        fee_bounds: Some(vec![WorkflowFeeBound {
            payer: acct(&owner),
            asset,
            max_fee: 50000u64.into(),
        }]),
        evidence: vec![],
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let s = TrustedState {
        contract_version: None,
        policy_state: None,
        source: AccountState {
            address: owner.clone(),
            sequence: 40,
            master_weight: 3,
            low_threshold: 2,
            medium_threshold: 2,
            high_threshold: 3,
            signers: vec![
                AccountSigner {
                    key: agent.clone(),
                    weight: 1,
                },
                AccountSigner {
                    key: engine.clone(),
                    weight: 1,
                },
            ],
        },
        nonce: 9,
        deployment_version: 4,
        spent_daily: 0,
        spent_total: 0,
        spent_day: now / 86400,
        calls: 0,
        ledger: 100,
        module_digest: c.module_digest,
        source_hash: c.source_hash,
        owner,
        agent,
        server: engine,
        policy_instance: c.policy_instance,
    };
    (c, r, s, now)
}
fn expires(now: u64) -> u64 {
    (now + 120).min((now / 86400 + 1) * 86400 - 1)
}

/// The public wallet profile: no signing material.
fn profile(c: &WalletConfig) -> Value {
    let mut value = json!({"version":1,"profile":PROFILE,"network":c.network,"policyId":POLICY,
        "sourceHash":sdk::hex(&c.source_hash),"irHash":sdk::hex(&[4;32]),
        "moduleDigest":sdk::hex(&c.module_digest),"wallet":c.wallet,"route":c.route,"config":c,
        "endpoints":{"execute":"/api/native/execute","status":"/api/native/execute/status"}});
    if c.language == sdk::PolicyLanguage::Portable {
        let mut checked = c.clone();
        value["portable"] = common::portable_descriptor(&mut checked);
        assert_eq!(checked.module_digest, c.module_digest);
        value["language"] = json!("allowit-portable-v1");
        value["irHash"] = json!("");
    }
    if c.route == WalletRoute::Native && c.language == sdk::PolicyLanguage::Portable {
        let b = allowit_sdk_engine::vendors::stellar::runtime::client::decode_public_bundle(
            &serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        let message = b.source_approval_message(1).unwrap();
        let key = sdk::signing::LocalSigner::from_entropy(&[1; 32]).unwrap();
        assert_eq!(key.account(), c.owner);
        value["sourceApproval"] = json!({"revision":1,"signatures":[{"signer":key.account(),"signature":key.sign_digest(&sdk::signing::signed_message_digest(&message)).unwrap()}]});
    }
    value
}

/// A sandbox on stellar:testnet with the agent key for `seed` set up, the
/// public profile installed and the mock engine as the engine URL.
fn setup(c: &WalletConfig, seed: u8, mock: &Mock) -> Sandbox {
    let mut sandbox = Sandbox::new("stellar");
    let key = sandbox.private_file("agent.key", &common::stellar_seed([seed; 32]));
    sandbox.ok(&["rail", "use", "stellar:testnet"]);
    sandbox.ok(&["keys", "import", "agent", key.to_str().unwrap()]);
    std::fs::remove_file(key).unwrap();
    sandbox.ok(&["keys", "use", "agent"]);
    let path = sandbox.work().join("profile.json");
    std::fs::write(&path, profile(c).to_string()).unwrap();
    common::public_profile_fixture(&sandbox, &profile(c), &mock.url);
    sandbox.set("ALLOWIT_URL", &mock.url);
    sandbox.ok(&["rail", "trust", &mock.url, &sdk::account_for_seed(&[3; 32])]);
    let chain = chain_fixture::Chain::new(c.clone());
    *chain.source.lock().unwrap() = chain_fixture::configured_source(c);
    chain.source.lock().unwrap().sequence = 40;
    let rpc = chain_fixture::RpcServer::new(chain);
    let path = sandbox.config().join("config.toml");
    let mut config: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["rails"]["stellar:testnet"]
        .as_table_mut()
        .unwrap()
        .insert("rpc_url".into(), toml::Value::String(rpc.url.clone()));
    std::fs::write(path, toml::to_string(&config).unwrap()).unwrap();
    sandbox.keep_alive(rpc);
    sandbox
}

fn exec_args() -> Vec<String> {
    [
        "exec",
        "--op",
        "transfer",
        "--to",
        &dest(),
        "--amount",
        "XLM:0.1",
        "--request-id",
        ID,
        "--wait",
        "0",
    ]
    .map(String::from)
    .to_vec()
}
fn exec(sandbox: &Sandbox) -> std::process::Output {
    let args = exec_args();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    sandbox.run(&args)
}
fn status(sandbox: &Sandbox) -> std::process::Output {
    sandbox.run(&["status", ID, "--wait", "0"])
}

fn journal_file(sandbox: &Sandbox, request: &ExecutionRequest) -> PathBuf {
    sandbox.config().join("stellar-journal").join(format!(
        "request-{}.json",
        sdk::hex(&sdk::hash(sdk::hex(&request.operation.nonce)))
    ))
}
/// Retains an unsigned prepared packet in the journal, as an interrupted
/// exec leaves it.
fn retain_unsigned(
    sandbox: &Sandbox,
    c: &WalletConfig,
    r: &ExecutionRequest,
    state: &TrustedState,
    now: u64,
) {
    use std::os::unix::fs::PermissionsExt;
    let prepared = sdk::prepare(c, r, state, now, expires(now)).unwrap();
    let file = journal_file(sandbox, r);
    let dir = file.parent().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    common::private_write(
        &file,
        &json!({"policyId":POLICY,"requestId":ID,"config":c,"request":r,"prepared":prepared,
            "agentSignature":null,"status":"prepared"})
        .to_string(),
    );
}

fn read(stream: &mut TcpStream) -> (String, Value) {
    read_for(stream, &sdk::account_for_seed(&[2; 32]))
}
/// Reads one engine request and verifies the agent's request proof.
fn read_for(stream: &mut TcpStream, expected_signer: &str) -> (String, Value) {
    let (h, body) = read_request(stream);
    let proof = h
        .lines()
        .find_map(|l| l.strip_prefix("authorization: Bearer "))
        .unwrap();
    let parts: Vec<_> = proof.split('.').collect();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0], "stellar-proof");
    assert_eq!(parts[1], expected_signer);
    let path = h.lines().next().unwrap().split(' ').nth(1).unwrap();
    let mut message = format!("allowit-stellar-request-v1\n{}\n{path}\n", parts[2]).into_bytes();
    message.extend(serde_json_canonicalizer::to_vec(&body).unwrap());
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    VerifyingKey::from_bytes(&sdk::account_key(parts[1]).unwrap())
        .unwrap()
        .verify(
            &sdk::hash(message),
            &Signature::from_bytes(&sdk::unhex::<64>(parts[3]).unwrap()),
        )
        .unwrap();
    let seed = common::stellar_seed([2; 32]);
    assert!(!h.contains(&seed));
    assert!(!body.to_string().contains(&seed));
    (h, body)
}
fn ok(s: &mut TcpStream, v: Value) {
    reply(s, 200, &v);
}
fn reply_error(stream: &mut TcpStream, status: u16) {
    reply(
        stream,
        status,
        &json!({"error":"fixture rejected before signing"}),
    );
}
fn response(p: &PreparedOperation, s: Option<&SignedOperation>, status: &str) -> Value {
    let receipt = if status == "confirmed" {
        s.map(|signed| {
            let result = TransactionResult {
                fee_charged: 100,
                result: TransactionResultResult::TxSuccess(
                    vec![OperationResult::OpInner(OperationResultTr::Payment(
                        PaymentResult::Success,
                    ))]
                    .try_into()
                    .unwrap(),
                ),
                ext: TransactionResultExt::V0,
            };
            sdk::rpc::VerifiedReceipt {
                transaction_hash: signed.transaction_hash,
                status: OperationStatus::Confirmed,
                ledger: 777,
                envelope_xdr: signed.transaction_xdr.clone(),
                result_xdr: result.to_xdr_base64(Limits::none()).unwrap(),
            }
        })
    } else {
        None
    };
    json!({"prepared":p,"operation":{"id":ID,"status":status,"transactionHash":sdk::hex(&p.transaction_hash)},"signedOperation":s,"receipt":receipt})
}
fn signed_from_submit(
    stream: &mut TcpStream,
    prepared: &PreparedOperation,
    now: u64,
) -> SignedOperation {
    let (head, body) = read(stream);
    assert!(head.starts_with("POST /api/native/execute "));
    assert_eq!(body["policyId"], POLICY);
    assert_eq!(body["requestId"], ID);
    assert_eq!(body["phase"], "submit");
    assert_eq!(body.as_object().unwrap().len(), 4);
    let agent: DetachedSignature = serde_json::from_value(body["agentSignature"].clone()).unwrap();
    sdk::authorize(
        prepared,
        &[agent, sdk::sign(prepared, &[3; 32], now).unwrap()],
        now,
    )
    .unwrap()
}

#[test]
fn two_phase_exec_verifies_real_signatures_and_unknown_retry_only_recovers() {
    let (c, r, s, now) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    let listener = mock.listener;
    let request = r.clone();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let (h, b) = read(&mut conn);
        assert!(h.starts_with("POST /api/native/execute "));
        // The flags built exactly the canonical typed request.
        assert_eq!(
            b,
            json!({"policyId":POLICY,"requestId":ID,"phase":"prepare","request":request})
        );
        let prepared = sdk::prepare(&c, &request, &s, now, expires(now)).unwrap();
        ok(&mut conn, response(&prepared, None, "prepared"));
        let (mut conn, _) = listener.accept().unwrap();
        let (_, b) = read(&mut conn);
        let signature: DetachedSignature =
            serde_json::from_value(b["agentSignature"].clone()).unwrap();
        assert_eq!(b.as_object().unwrap().len(), 4);
        assert_eq!(b["phase"], "submit");
        let engine = sdk::sign(&prepared, &[3; 32], now).unwrap();
        let signed = sdk::authorize(&prepared, &[signature, engine], now).unwrap();
        sdk::validate_signed(&signed, now).unwrap();
        drop(conn); // The broadcast receipt is lost.
        let (mut conn, _) = listener.accept().unwrap();
        let (h, b) = read(&mut conn);
        assert!(h.starts_with("POST /api/native/execute/status "));
        assert_eq!(b, json!({"policyId":POLICY,"requestId":ID}));
        ok(&mut conn, response(&prepared, Some(&signed), "confirmed"));
    });
    let first = exec(&sandbox);
    assert_eq!(code(&first), Some(5), "{}", stderr(&first));
    let recovered = exec(&sandbox);
    assert_eq!(code(&recovered), Some(0), "{}", stderr(&recovered));
    let output = String::from_utf8_lossy(&recovered.stdout);
    assert!(output.contains("confirmed"));
    common::assert_data(&output, "stellar exec");
    common::assert_data(&stderr(&recovered), "stellar exec");
    let seed = common::stellar_seed([2; 32]);
    assert!(!output.contains(&seed));
    for dir in ["stellar-journal", "requests"] {
        for entry in std::fs::read_dir(sandbox.config().join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                assert!(!std::fs::read_to_string(path).unwrap().contains(&seed));
            }
        }
    }
    // The request journal records the request and its last exit code.
    let history = sandbox.json(&["history"]);
    assert_eq!(history[0]["requestId"], ID);
    assert_eq!(history[0]["rail"], "stellar:testnet");
    assert_eq!(history[0]["exit"], 0);
    server.join().unwrap();
}

#[test]
fn build_only_prints_the_typed_request_and_sends_nothing() {
    let (c, r, _, _) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    let mut args = exec_args();
    args.push("--build-only".into());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = sandbox.run(&args);
    assert_eq!(code(&output), Some(0), "{}", stderr(&output));
    let wire: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        wire,
        json!({"requestId":ID,"input":{"kind":"execution-request","request":r}})
    );
    mock.assert_untouched();
    assert!(!sandbox.config().join("requests").exists());
}

#[test]
fn changed_prepared_request_never_reaches_signing_or_submit() {
    let (c, r, s, now) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        read(&mut conn);
        let p = sdk::prepare(&c, &r, &s, now, expires(now)).unwrap();
        let mut v = response(&p, None, "prepared");
        v["prepared"]["request"]["operation"]["nonce"][0] = json!(99);
        ok(&mut conn, v);
    });
    let output = exec(&sandbox);
    assert_eq!(code(&output), Some(5));
    assert!(stderr(&output).contains("identity"));
    server.join().unwrap();
}

#[test]
fn malformed_profiles_and_flags_fail_before_network() {
    let (c, _, _, _) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    let path = sandbox.work().join("profile.json");
    let original = std::fs::read(&path).unwrap();
    common::public_profile_fixture(&sandbox, &json!({"network":"stellar:testnet"}), &mock.url);
    let output = exec(&sandbox);
    assert_eq!(code(&output), Some(3), "{}", stderr(&output));
    assert!(stderr(&output).contains("public wallet profile"));
    let mut proof_free: Value = serde_json::from_slice(&original).unwrap();
    proof_free.as_object_mut().unwrap().remove("sourceApproval");
    common::public_profile_fixture(&sandbox, &proof_free, &mock.url);
    let refused = exec(&sandbox);
    assert_eq!(code(&refused), Some(3), "{}", stderr(&refused));
    assert!(
        stderr(&refused).contains("source approval"),
        "{}",
        stderr(&refused)
    );
    common::public_profile_fixture(
        &sandbox,
        &serde_json::from_slice(&original).unwrap(),
        &mock.url,
    );
    let to = dest();
    for (args, expected) in [
        (vec!["exec", "--op", "transfer", "--action", "transfer"], 2),
        (
            vec!["exec", "--op", "transfer", "--to", "GBAD", "--amount", "1"],
            2,
        ),
        (
            vec![
                "exec",
                "--op",
                "transfer",
                "--to",
                &to,
                "--amount",
                "XLM:1.00000001",
            ],
            2,
        ),
        (
            vec![
                "exec", "--op", "transfer", "--to", &to, "--amount", "XLM:max",
            ],
            2,
        ),
        (
            vec!["exec", "--op", "transfer", "--to", &to, "--amount", "0"],
            2,
        ),
        (
            vec!["exec", "--op", "swap", "--to", &to, "--amount", "XLM:1"],
            3,
        ),
        (
            vec!["exec", "--op", "steal", "--to", &to, "--amount", "XLM:1"],
            2,
        ),
        (
            vec![
                "exec",
                "--op",
                "transfer",
                "--to",
                &to,
                "--amount",
                "XLM:1",
                "--request-id",
                "short",
            ],
            2,
        ),
        (
            vec![
                "exec",
                "--op",
                "transfer",
                "--to",
                &to,
                "--amount",
                "XLM:1",
                "--token-out",
                "USDC:1",
            ],
            2,
        ),
        // The wallet holds only its installed asset.
        (
            vec![
                "exec", "--op", "transfer", "--to", &to, "--amount", "USDC:1",
            ],
            4,
        ),
    ] {
        let output = sandbox.run(&args);
        assert_eq!(
            code(&output),
            Some(expected),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(output.stdout.is_empty(), "{args:?}");
    }
    mock.assert_untouched();
}

#[test]
fn substituted_receipt_never_becomes_a_confirmed_cli_result() {
    let (c, r, s, now) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        read(&mut conn);
        let p = sdk::prepare(&c, &r, &s, now, expires(now)).unwrap();
        let mut value = response(&p, None, "prepared");
        value["receipt"] = json!({"transactionHash":vec![0u8;32],"status":"confirmed","ledger":777,"envelopeXdr":"substituted","resultXdr":"substituted"});
        ok(&mut conn, value);
    });
    let output = exec(&sandbox);
    assert_eq!(code(&output), Some(3));
    assert!(stderr(&output).contains("receipt"));
    assert!(output.stdout.is_empty());
    server.join().unwrap();
}

#[test]
fn signed_packet_without_terminal_receipt_never_proves_settlement() {
    let (c, r, s, now) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let (head, body) = read(&mut conn);
        assert!(head.starts_with("POST /api/native/execute/status "));
        assert_eq!(body["requestId"], ID);
        let prepared = sdk::prepare(&c, &r, &s, now, expires(now)).unwrap();
        let signed = sdk::authorize(
            &prepared,
            &[
                sdk::sign(&prepared, &[2; 32], now).unwrap(),
                sdk::sign(&prepared, &[3; 32], now).unwrap(),
            ],
            now,
        )
        .unwrap();
        let mut value = response(&prepared, Some(&signed), "confirmed");
        value["receipt"] = Value::Null;
        ok(&mut conn, value);
    });
    let output = status(&sandbox);
    assert_eq!(code(&output), Some(5));
    assert!(stderr(&output).contains("verified chain receipt"));
    assert!(output.stdout.is_empty());
    server.join().unwrap();
}

#[test]
fn never_prepared_intent_retries_only_same_identity_after_status_404() {
    let (c, request, state, now) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        let (head, original) = read(&mut first);
        assert!(head.starts_with("POST /api/native/execute "));
        assert_eq!(original["phase"], "prepare");
        reply_error(&mut first, 409);
        let (mut status, _) = listener.accept().unwrap();
        let (head, body) = read(&mut status);
        assert!(head.starts_with("POST /api/native/execute/status "));
        assert_eq!(body, json!({"policyId":POLICY,"requestId":ID}));
        reply_error(&mut status, 404);
        let (mut retry, _) = listener.accept().unwrap();
        let (_, body) = read(&mut retry);
        assert_eq!(body, original);
        let prepared = sdk::prepare(&c, &request, &state, now, expires(now)).unwrap();
        ok(&mut retry, response(&prepared, None, "prepared"));
        let (mut submit, _) = listener.accept().unwrap();
        let signed = signed_from_submit(&mut submit, &prepared, now);
        ok(&mut submit, response(&prepared, Some(&signed), "confirmed"));
    });
    assert_eq!(code(&exec(&sandbox)), Some(4));
    let out = exec(&sandbox);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    server.join().unwrap();
}

#[test]
fn unsigned_prepared_resumes_exact_packet_only_on_explicit_exec() {
    let (c, request, state, now) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    retain_unsigned(&sandbox, &c, &request, &state, now);
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        let prepared = sdk::prepare(&c, &request, &state, now, expires(now)).unwrap();
        for _ in 0..2 {
            let (mut status, _) = listener.accept().unwrap();
            let (head, body) = read(&mut status);
            assert!(head.starts_with("POST /api/native/execute/status "));
            assert_eq!(body, json!({"policyId":POLICY,"requestId":ID}));
            ok(&mut status, response(&prepared, None, "prepared"));
        }
        let (mut submit, _) = listener.accept().unwrap();
        let signed = signed_from_submit(&mut submit, &prepared, now);
        ok(&mut submit, response(&prepared, Some(&signed), "confirmed"));
    });
    // status has no journal entry for the ID, so it uses the current rail.
    assert_eq!(code(&status(&sandbox)), Some(12));
    let out = exec(&sandbox);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    server.join().unwrap();
}

#[test]
fn retained_unsigned_prepared_is_never_replaced_after_status_404() {
    let (c, request, state, now) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    retain_unsigned(&sandbox, &c, &request, &state, now);
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        let (mut status, _) = listener.accept().unwrap();
        let (head, _) = read(&mut status);
        assert!(head.starts_with("POST /api/native/execute/status "));
        reply_error(&mut status, 404);
    });
    assert_eq!(code(&exec(&sandbox)), Some(4));
    server.join().unwrap();
}

#[test]
fn unsigned_resume_rejects_changed_server_packet_before_signing() {
    let (c, request, state, now) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    retain_unsigned(&sandbox, &c, &request, &state, now);
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        let mut changed = state;
        changed.source.sequence += 1;
        let changed = sdk::prepare(&c, &request, &changed, now, expires(now)).unwrap();
        let (mut status, _) = listener.accept().unwrap();
        let (head, _) = read(&mut status);
        assert!(head.starts_with("POST /api/native/execute/status "));
        ok(&mut status, response(&changed, None, "prepared"));
    });
    let out = exec(&sandbox);
    assert_eq!(code(&out), Some(5));
    assert!(stderr(&out).contains("identity"));
    server.join().unwrap();
}

#[test]
fn imported_unknown_signed_operation_never_authorizes_from_unsigned_local_state() {
    let (c, request, state, now) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        let prepared = sdk::prepare(&c, &request, &state, now, expires(now)).unwrap();
        let signed = sdk::authorize(
            &prepared,
            &[
                sdk::sign(&prepared, &[2; 32], now).unwrap(),
                sdk::sign(&prepared, &[3; 32], now).unwrap(),
            ],
            now,
        )
        .unwrap();
        for status in ["submissionUnknown", "confirmed"] {
            let (mut conn, _) = listener.accept().unwrap();
            let (head, body) = read(&mut conn);
            assert!(head.starts_with("POST /api/native/execute/status "));
            assert_eq!(body, json!({"policyId":POLICY,"requestId":ID}));
            ok(&mut conn, response(&prepared, Some(&signed), status));
        }
    });
    assert_eq!(code(&status(&sandbox)), Some(5));
    let out = exec(&sandbox);
    assert_eq!(code(&out), Some(0), "{}", stderr(&out));
    server.join().unwrap();
}

#[test]
fn expired_unsigned_packet_observation_prevents_repreparation_after_404() {
    let (c, request, mut state, now) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&c, 2, &mock);
    let listener = mock.listener;
    let server = std::thread::spawn(move || {
        let created = (now / 86400 - 1) * 86400 + 1;
        state.spent_day = created / 86400;
        let prepared = sdk::prepare(&c, &request, &state, created, created + 120).unwrap();
        let (mut first, _) = listener.accept().unwrap();
        read(&mut first);
        drop(first);
        let (mut status, _) = listener.accept().unwrap();
        let (head, _) = read(&mut status);
        assert!(head.starts_with("POST /api/native/execute/status "));
        ok(&mut status, response(&prepared, None, "prepared"));
        let (mut status, _) = listener.accept().unwrap();
        let (head, _) = read(&mut status);
        assert!(head.starts_with("POST /api/native/execute/status "));
        reply_error(&mut status, 404);
    });
    assert_eq!(code(&exec(&sandbox)), Some(5));
    let expired = exec(&sandbox);
    assert_eq!(code(&expired), Some(3));
    assert!(stderr(&expired).contains("stale"));
    assert_eq!(code(&exec(&sandbox)), Some(4));
    server.join().unwrap();
}

#[test]
fn replacement_agent_key_recovers_original_receipt_and_exact_journal_without_signing() {
    for changed_source in [false, true] {
        let (config, request, state, now) = fixture();
        let prepared = sdk::prepare(&config, &request, &state, now, expires(now)).unwrap();
        let agent = sdk::sign(&prepared, &[2; 32], now).unwrap();
        let signed = sdk::authorize(
            &prepared,
            &[agent.clone(), sdk::sign(&prepared, &[3; 32], now).unwrap()],
            now,
        )
        .unwrap();
        let mock = Mock::new();
        let sandbox = setup(&config, 2, &mock);
        retain_unsigned(&sandbox, &config, &request, &state, now);
        let file = journal_file(&sandbox, &request);
        let mut original: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        original["agentSignature"] = json!(agent);
        original["status"] = json!("submissionUnknown");
        common::private_write(&file, &original.to_string());
        // The owner rotates the agent: a new key and a profile that names it.
        let key = sandbox.private_file("agent9.key", &common::stellar_seed([9; 32]));
        sandbox.ok(&["keys", "import", "agent9", key.to_str().unwrap()]);
        sandbox.ok(&["keys", "use", "agent9"]);
        let mut current = config.clone();
        current.agent = sdk::account_for_seed(&[9; 32]);
        if changed_source {
            current.policy_source.push_str("\n// revised source\n");
            current.source_hash = sdk::hash(&current.policy_source);
            common::portable_descriptor(&mut current);
            let shard = allowit_sdk::digest(ID.as_bytes())[..2].to_owned();
            let ux = allowit_sdk::client::store::Store::open(
                &sandbox.config().join("requests").join(shard),
            )
            .unwrap();
            ux.save(ID,&json!({"requestId":ID,"command":"exec","rail":config.network,"wallet":config.wallet,"policy":format!("{}@{}",POLICY,sdk::hex(&config.module_digest)),"op":"transfer","to":dest(),"amount":"XLM:0.1","createdAt":now,"exit":5})).unwrap();
        }
        let rotated = profile(&current);
        common::public_profile_fixture(&sandbox, &rotated, &mock.url);
        // Prior-release journals have no public cache envelope or new authority pin.
        // Recovery must observe the retained operation without fresh admission.
        std::fs::remove_dir_all(sandbox.config().join("public-profiles")).unwrap();
        let settings = sandbox.config().join("config.toml");
        let mut values: toml::Value =
            toml::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        values["rails"]["stellar:testnet"]
            .as_table_mut()
            .unwrap()
            .remove("server_authorities");
        std::fs::write(settings, toml::to_string(&values).unwrap()).unwrap();
        let listener = mock.listener;
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut connection, _) = listener.accept().unwrap();
                let (head, body) = read_for(&mut connection, &sdk::account_for_seed(&[9; 32]));
                assert!(head.starts_with("POST /api/native/execute/status "));
                assert_eq!(body, json!({"policyId":POLICY,"requestId":ID}));
                ok(
                    &mut connection,
                    response(&prepared, Some(&signed), "confirmed"),
                );
            }
        });
        let shown = status(&sandbox);
        assert_eq!(code(&shown), Some(0), "{}", stderr(&shown));
        let recovered = exec(&sandbox);
        assert_eq!(code(&recovered), Some(0), "{}", stderr(&recovered));
        let after: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(after["prepared"], original["prepared"]);
        assert_eq!(after["agentSignature"], original["agentSignature"]);
        assert_eq!(after["config"], original["config"]);
        assert_eq!(after["request"], original["request"]);
        server.join().unwrap();
    }
}

#[test]
fn status_uses_the_journaled_rail_and_refuses_another() {
    let (c, r, s, now) = fixture();
    let mock = Mock::new();
    let mut sandbox = setup(&c, 2, &mock);
    let listener = mock.listener;
    let config = c.clone();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        read(&mut conn);
        reply_error(&mut conn, 409);
        let (mut conn, _) = listener.accept().unwrap();
        let (head, _) = read(&mut conn);
        assert!(head.starts_with("POST /api/native/execute/status "));
        let prepared = sdk::prepare(&config, &r, &s, now, expires(now)).unwrap();
        ok(&mut conn, response(&prepared, None, "prepared"));
        let (mut conn, _) = listener.accept().unwrap();
        let (head, body) = read(&mut conn);
        assert!(head.starts_with("POST /api/native/execute/status "));
        assert_eq!(body, json!({"policyId":POLICY,"requestId":ID}));
        ok(&mut conn, response(&prepared, None, "prepared"));
    });
    assert_eq!(code(&exec(&sandbox)), Some(4));
    // Another current rail does not change where status looks: neither
    // `rail use` nor ALLOWIT_RAIL conflicts with the journal.
    sandbox.ok(&["rail", "use", "stellar:pubnet"]);
    sandbox.set("ALLOWIT_RAIL", "stellar:pubnet");
    let shown = status(&sandbox);
    assert_eq!(code(&shown), Some(12), "{}", stderr(&shown));
    let refused = sandbox.run(&["--rail", "stellar:pubnet", "status", ID]);
    assert_eq!(code(&refused), Some(2));
    assert!(stderr(&refused).contains("belongs to stellar:testnet"));
    // The journal binds the request ID to the resolved wallet and policy:
    // after the installed profile changes, the same ID neither sends a new
    // request nor reads another policy's state.
    let mut moved = profile(&c);
    moved["policyId"] = json!("another-policy");
    common::public_profile_fixture(&sandbox, &moved, &mock.url);
    sandbox.set("ALLOWIT_RAIL", "stellar:testnet");
    let retry = exec(&sandbox);
    assert_eq!(code(&retry), Some(2), "{}", stderr(&retry));
    assert!(
        stderr(&retry).contains("already names a different request"),
        "{}",
        stderr(&retry)
    );
    let shown = status(&sandbox);
    assert_eq!(code(&shown), Some(12), "{}", stderr(&shown));
    // Status read the original policy; it did not admit the selected replacement.
    server.join().unwrap();
}

#[test]
fn selected_key_must_match_the_wallet_agent_before_transport_or_signing() {
    let (config, _, _, _) = fixture();
    let mock = Mock::new();
    let sandbox = setup(&config, 9, &mock);
    let refused = exec(&sandbox);
    assert_eq!(code(&refused), Some(3), "{}", stderr(&refused));
    assert!(stderr(&refused).contains("No verified public profile matches"));
    mock.assert_untouched();
}
