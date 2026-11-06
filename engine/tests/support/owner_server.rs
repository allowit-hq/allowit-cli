//! Independent replay of the current server HTTP contract, not a live generation or build proof.
#![allow(dead_code)]
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn fixture() -> Value {
    allowit_stellar_wallet::test_support::owner_generation_fixture()
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
pub fn challenge(owner: &str, origin: &str, now: u64) -> Value {
    allowit_stellar_wallet::test_support::server::owner_signin_challenge(owner, origin, now)
}
pub struct Plan {
    documents: allowit_stellar_wallet::test_support::server::OwnerDocuments,
    pub guest_cookie: String,
    pub rotate_cookie: bool,
    pub lost_generation: bool,
    pub redirect_challenge: Option<String>,
    pub redirect_verify: Option<String>,
    pub owner_prepare: Option<Value>,
    pub owner_next_prepare: Option<Value>,
    pub lose_owner_reply: bool,
    pub approval: Option<Value>,
    pub activation: Option<Value>,
    pub export: Option<Value>,
}
impl std::ops::Deref for Plan {
    type Target = allowit_stellar_wallet::test_support::server::OwnerDocuments;
    fn deref(&self) -> &Self::Target {
        &self.documents
    }
}
impl std::ops::DerefMut for Plan {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.documents
    }
}
impl Plan {
    pub fn new(owner: &str) -> Self {
        Self {
            documents: allowit_stellar_wallet::test_support::server::OwnerDocuments::new(
                owner,
                "http://localhost:5173",
                now(),
            ),
            guest_cookie: format!(
                "allowit_session={}; Path=/; HttpOnly; SameSite=Lax",
                "g".repeat(32)
            ),
            rotate_cookie: true,
            lost_generation: false,
            redirect_challenge: None,
            redirect_verify: None,
            owner_prepare: None,
            owner_next_prepare: None,
            lose_owner_reply: false,
            approval: None,
            activation: None,
            export: None,
        }
    }
}
#[derive(Default)]
pub struct State {
    pub calls: BTreeMap<String, usize>,
    pub generated: Vec<Value>,
    pub owner_submissions: Vec<Value>,
}
pub struct Server {
    pub url: String,
    pub state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}
fn input(stream: &mut TcpStream) -> (String, BTreeMap<String, String>, Value) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 16_384);
        let mut b = [0];
        stream.read_exact(&mut b).unwrap();
        bytes.push(b[0]);
    }
    let raw = String::from_utf8(bytes).unwrap();
    let mut lines = raw.lines();
    let path = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    let headers: BTreeMap<_, _> = lines
        .filter_map(|v| {
            v.split_once(':')
                .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
        })
        .collect();
    let length = headers
        .get("content-length")
        .map_or(0, |s| s.parse::<usize>().unwrap());
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    (
        path,
        headers,
        if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        },
    )
}
fn reply(stream: &mut TcpStream, status: u16, body: Value, headers: &str) {
    let bytes = serde_json::to_vec(&body).unwrap();
    write!(stream,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",bytes.len()).unwrap();
    stream.write_all(&bytes).unwrap();
}
impl Server {
    pub fn new(plan: Plan) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let captured = state.clone();
        let quitting = stop.clone();
        let task = thread::spawn(move || {
            while !quitting.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                let (path, headers, body) = input(&mut stream);
                *captured
                    .lock()
                    .unwrap()
                    .calls
                    .entry(path.clone())
                    .or_default() += 1;
                assert_eq!(headers.get("origin"), Some(&plan.origin));
                assert!(!headers.contains_key("authorization"));
                let guest = format!("allowit_session={}", "g".repeat(32));
                let owner = format!("allowit_session={}", "s".repeat(32));
                if path == "/api/auth/challenge" {
                    assert_eq!(body, json!({"address":plan.owner}));
                    assert!(!headers.contains_key("cookie"));
                    if let Some(location) = &plan.redirect_challenge {
                        reply(
                            &mut stream,
                            302,
                            json!({}),
                            &format!("Location: {location}\r\n"),
                        );
                        continue;
                    }
                    reply(
                        &mut stream,
                        200,
                        plan.challenge.clone(),
                        &format!("Set-Cookie: {}\r\n", plan.guest_cookie),
                    );
                    continue;
                }
                if path == "/api/auth/verify" {
                    assert_eq!(headers.get("cookie"), Some(&guest));
                    assert_eq!(body["message"], plan.challenge["message"]);
                    assert_eq!(body["id"], plan.challenge["id"]);
                    assert_eq!(body["address"], plan.owner);
                    assert!(
                        allowit_stellar_wallet::test_support::verify_owner_challenge(
                            &plan.owner,
                            body["message"].as_str().unwrap(),
                            body["signature"].as_str().unwrap(),
                        )
                    );
                    if let Some(location) = &plan.redirect_verify {
                        reply(
                            &mut stream,
                            302,
                            json!({}),
                            &format!("Location: {location}\r\n"),
                        );
                        continue;
                    }
                    let cookie = if plan.rotate_cookie { &owner } else { &guest };
                    reply(
                        &mut stream,
                        200,
                        allowit_stellar_wallet::test_support::server::owner_session(
                            &plan.owner,
                            now() + 86400,
                        ),
                        &format!("Set-Cookie: {cookie}; Path=/; HttpOnly; SameSite=Lax\r\n"),
                    );
                    continue;
                }
                assert_eq!(headers.get("cookie"), Some(&owner));
                match path.as_str() {
                    "/api/native/approval" => reply(
                        &mut stream,
                        200,
                        plan.approval.clone().expect("approval fixture"),
                        "",
                    ),
                    "/api/micro/allocation/confirm" => reply(
                        &mut stream,
                        200,
                        plan.activation.clone().expect("activation fixture"),
                        "",
                    ),
                    "/api/native/export" => reply(
                        &mut stream,
                        200,
                        plan.export.clone().expect("public export fixture"),
                        "",
                    ),
                    "/api/native/owner/prepare" | "/api/native/deploy/prepare" => {
                        reply(
                            &mut stream,
                            200,
                            if captured.lock().unwrap().owner_submissions.is_empty() {
                                plan.owner_prepare.clone()
                            } else {
                                plan.owner_next_prepare
                                    .clone()
                                    .or_else(|| plan.owner_prepare.clone())
                            }
                            .expect("owner preparation fixture"),
                            "",
                        );
                    }
                    "/api/native/owner/submit" => {
                        let signed = body["transactionXdr"].as_str().unwrap();
                        captured
                            .lock()
                            .unwrap()
                            .owner_submissions
                            .push(body.clone());
                        if plan.lose_owner_reply
                            && captured.lock().unwrap().owner_submissions.len() == 1
                        {
                            continue;
                        }
                        reply(
                            &mut stream,
                            200,
                            allowit_stellar_wallet::test_support::server::confirmed_owner_submission("stellar:testnet",signed),
                            "",
                        );
                    }
                    "/api/micro/runtime" => reply(&mut stream, 200, plan.discovery.clone(), ""),
                    "/api/native/templates" => {
                        assert_eq!(body, json!({}));
                        reply(&mut stream, 200, plan.templates.clone(), "")
                    }
                    "/api/native/policies" => {
                        reply(&mut stream, 200, plan.installations.clone(), "")
                    }
                    "/api/micro/generate" => {
                        captured.lock().unwrap().generated.push(body.clone());
                        if plan.lost_generation {
                            continue;
                        }
                        reply(
                            &mut stream,
                            200,
                            allowit_stellar_wallet::test_support::server::generated_policy_reply(
                                &plan.policy,
                                &body["prompt"],
                            ),
                            "",
                        )
                    }
                    _ => panic!("unexpected owner route {path}"),
                }
            }
        });
        Self {
            url,
            state,
            stop,
            task: Some(task),
        }
    }
    pub fn calls(&self, path: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .calls
            .get(path)
            .copied()
            .unwrap_or(0)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.task.take().unwrap().join();
        if !thread::panicking() {
            result.expect("owner protocol fixture server failed");
        }
    }
}
