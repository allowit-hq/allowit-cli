//! Shared helpers for the process tests: an isolated sandbox per test and a
//! one-connection-at-a-time mock engine.
#![allow(dead_code)]
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

/// A private directory with a config directory and a working directory.
/// Commands run with every `ALLOWIT_*` variable removed, no color, a fixed
/// help width and proxies that refuse every host except the loopback.
pub struct Sandbox {
    pub root: PathBuf,
    env: Vec<(String, String)>,
    services: Vec<Box<dyn std::any::Any>>,
}

impl Sandbox {
    pub fn new(name: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "allowit-cli-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("work")).unwrap();
        Self {
            root,
            env: vec![],
            services: vec![],
        }
    }
    pub fn keep_alive(&mut self, service: impl std::any::Any) {
        self.services.push(Box::new(service));
    }
    /// The config directory. Commands create it on first use.
    pub fn config(&self) -> PathBuf {
        self.root.join("config")
    }
    /// The working directory of every command.
    pub fn work(&self) -> PathBuf {
        self.root.join("work")
    }
    /// Sets an environment variable for every later command.
    pub fn set(&mut self, name: &str, value: impl AsRef<std::ffi::OsStr>) -> &mut Self {
        self.env.retain(|(n, _)| n != name);
        self.env
            .push((name.into(), value.as_ref().to_string_lossy().into_owned()));
        self
    }
    pub fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_allowit"));
        for (name, _) in std::env::vars_os() {
            let name = name.to_string_lossy().into_owned();
            if name.starts_with("ALLOWIT_") || name == "NO_DNA" || name == "CLICOLOR_FORCE" {
                c.env_remove(name);
            }
        }
        c.current_dir(self.work())
            .env("ALLOWIT_CONFIG_DIR", self.config())
            .env("NO_COLOR", "1")
            .env("COLUMNS", "100")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("ALLOWIT_URL", "http://127.0.0.1:1")
            .env("ALLOWIT_FRIENDBOT_URL", "http://127.0.0.1:1");
        for (name, value) in &self.env {
            c.env(name, value);
        }
        c
    }
    pub fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    /// Runs a command that must succeed and returns its stdout.
    pub fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert_eq!(
            output.status.code(),
            Some(0),
            "allowit {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    pub fn json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        serde_json::from_str(&self.ok(&all)).unwrap()
    }
    /// Writes a private (0600) file in the working directory.
    pub fn private_file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.work().join(name);
        private_write(&path, text);
        path
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub fn private_write(path: &Path, text: &str) {
    let _ = std::fs::remove_file(path);
    std::fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

pub fn code(output: &Output) -> Option<i32> {
    output.status.code()
}
pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A Stellar S strkey for a 32-byte seed.
pub fn stellar_seed(seed: [u8; 32]) -> String {
    let mut raw = vec![18 << 3];
    raw.extend(seed);
    let mut crc = 0u16;
    for b in &raw {
        crc ^= (*b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    raw.extend(crc.to_le_bytes());
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let (mut bits, mut count) = (0u32, 0);
    for b in raw {
        bits = (bits << 8) | b as u32;
        count += 8;
        while count >= 5 {
            count -= 5;
            out.push(ALPHABET[((bits >> count) & 31) as usize] as char);
        }
        bits &= (1 << count) - 1;
    }
    if count > 0 {
        out.push(ALPHABET[((bits << (5 - count)) & 31) as usize] as char);
    }
    out
}

/// A loopback listener that stands in for the engine or an RPC node.
pub struct Mock {
    pub listener: TcpListener,
    pub url: String,
}

impl Mock {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        Self { listener, url }
    }
    /// Asserts that no command has connected.
    pub fn assert_untouched(&self) {
        self.listener.set_nonblocking(true).unwrap();
        assert_eq!(
            self.listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "a command reached the network"
        );
        self.listener.set_nonblocking(false).unwrap();
    }
}

/// Reads one HTTP request: the head and the JSON body.
pub fn read_request(stream: &mut TcpStream) -> (String, Value) {
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut head = vec![];
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    let length = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    (head, body)
}

pub fn reply(stream: &mut TcpStream, status: u16, body: &Value) {
    let body = body.to_string();
    write!(
        stream,
        "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
}

/// True for one line of text data: a single value, `key  value`, or a table
/// row whose cells are separated by two or more spaces. A leading `* `
/// marks the selected row. Prose (words separated by single spaces before
/// the first cell ends) is not data.
pub fn data_line(line: &str) -> bool {
    let body = line
        .strip_prefix("* ")
        .or_else(|| line.strip_prefix("  "))
        .unwrap_or(line);
    let first = body.split("  ").next().unwrap_or_default();
    !first.is_empty() && !first.contains(char::is_whitespace)
}

/// Asserts that terminal text is data only: a JSON document, or data lines.
/// A bare key line may be followed by its indented multi-line value; a
/// blank line separates blocks.
pub fn assert_data(text: &str, what: &str) {
    if text.trim().is_empty() || serde_json::from_str::<Value>(text).is_ok() {
        return;
    }
    let mut block = false;
    for line in text.lines() {
        // A blank line separates two blocks of data.
        if line.is_empty() {
            block = false;
            continue;
        }
        if block && line.starts_with("  ") {
            continue;
        }
        assert!(
            data_line(line),
            "{what}: {line:?} is not a data line in\n{text}"
        );
        block = !line.contains(char::is_whitespace);
    }
}

/// Public descriptor metadata derived from a real portable configuration, never build proof.
pub fn portable_descriptor(config: &mut allowit_stellar_wallet::WalletConfig) -> Value {
    use allowit_stellar_wallet::{self as sdk, PolicyTier, WalletRoute};
    let shape = sdk::policy::source_shape(&config.policy_source).unwrap();
    let tier = if config.tier == Some(PolicyTier::Custom) {
        "custom"
    } else {
        "standard"
    };
    let rules = if tier == "standard" {
        shape.prefix.clone()
    } else {
        vec![]
    };
    let params_hash =
        allowit_sdk::digest(&serde_json_canonicalizer::to_vec(&config.params).unwrap());
    let identity = serde_json::json!({"language":"allowit-portable-v1","tier":tier,"rules":rules,"sourceHash":sdk::hex(&config.source_hash),"paramsHash":params_hash});
    if config.route == WalletRoute::Native {
        config.module_digest = sdk::unhex(&allowit_sdk::digest(
            &serde_json_canonicalizer::to_vec(&identity).unwrap(),
        ))
        .unwrap();
    }
    serde_json::json!({"language":"allowit-portable-v1","sourceHash":sdk::hex(&config.source_hash),"tier":tier,"rules":rules,"params":config.params,"paramsHash":params_hash,
        "asset":{"symbol":config.asset_symbol,"address":config.asset,"decimals":7},"unresolved":[],
        "build":{"prefix":shape.prefix,"writesState":shape.writes_state,"libraryRevision":"6aa4235e6229e18d1f9780a6a9fdf80aceb0f348","attempts":1}})
}

/// Seed an already verified export fixture. Production installation uses owner::export.
pub fn public_profile_fixture(sandbox: &Sandbox, profile: &Value, backend: &str) {
    let store =
        allowit_sdk::client::store::Store::open(&sandbox.config().join("public-profiles/versions"))
            .unwrap();
    store.save("fixture",&serde_json::json!({"schema":1,"network":profile["network"],"backend":backend,"descriptor":profile})).unwrap();
}
