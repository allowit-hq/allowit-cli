//! `allowit POLICY_ID help`: what the agent of one policy does next.
//!
//! The owner copies one line from the app. It holds the policy id and nothing
//! else. This command reads the public facts of that policy from the server
//! and prints the agent's next commands in order, with every value filled in.
//! It changes nothing: it pins no server key and imports no profile. The
//! server key that it prints is the server's own claim; the owner compares it
//! with the key that the app shows before the agent runs `rail trust`.
use crate::{
    ctx::Ctx,
    error::{Error, Result},
    rails::address,
    routing,
    settings::keys,
};
use allowit_sdk::transport::{
    http::{HttpClient, Method, Request},
    rpc::Options,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

/// Print what the agent of a policy does next, with the policy's real values
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// The policy id that the owner copied from the app
    #[arg(long, value_name = "POLICY_ID")]
    pub id: String,
}

/// The server's answer to `GET /api/agent/policy?id=POLICY_ID`. Unknown
/// fields are ignored, so a later server can add facts.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Facts {
    pub state: String,
    pub network: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub server_public_key: Option<String>,
    #[serde(default)]
    pub wallet_kind: Option<String>,
    #[serde(default)]
    pub agent_public_key: Option<String>,
    #[serde(default)]
    pub wallet: Option<String>,
    #[serde(default)]
    pub asset_symbol: Option<String>,
    #[serde(default)]
    pub limits: Vec<Limit>,
    #[serde(default)]
    pub profile: Option<Value>,
}

#[derive(Deserialize)]
pub(crate) struct Limit {
    pub key: String,
    pub label: String,
    pub value: String,
}

/// True for a policy id: the characters of a URL-safe identifier.
pub(crate) fn is_policy_id(text: &str) -> bool {
    (8..=128).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Text that the server sent and that this command prints as data: bounded,
/// one line and without control characters.
fn shown(text: &str, limit: usize) -> Result<String> {
    if text.is_empty() || text.len() > limit || text.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(text.to_owned())
}

/// A server value that this command prints inside a command line. Only
/// characters that no shell interprets are accepted.
fn shell_safe(text: &str, limit: usize) -> Result<String> {
    let plain = |b: u8| b.is_ascii_alphanumeric() || b"._~:/%@+-".contains(&b);
    if text.is_empty() || text.len() > limit || !text.bytes().all(plain) {
        return Err(invalid());
    }
    Ok(text.to_owned())
}

/// A local path as one shell word: bare when no shell interprets it, single-quoted otherwise.
fn shell_quote(text: &str) -> String {
    let plain = |b: u8| b.is_ascii_alphanumeric() || b"._~:/%@+-".contains(&b);
    if !text.is_empty() && text.bytes().all(plain) {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

fn invalid() -> Error {
    Error::uncertain("The server returned invalid policy facts")
}

/// Reads the public facts of `id` from the `ALLOWIT_URL` server. The request
/// carries no credential. Every value that the command prints is checked.
pub(crate) fn fetch(cx: &mut Ctx, id: &str) -> Result<(String, Facts)> {
    if !is_policy_id(id) {
        return Err(Error::usage(
            "the policy id has 8 to 128 letters, digits, - or _; copy it from the app",
        ));
    }
    let origin = crate::config::parse_origin(&cx.server_url()?)?;
    let url = format!("{origin}/api/agent/policy?id={id}");
    let client = HttpClient::new(Options {
        timeout: Duration::from_secs(20),
        max_response_bytes: 1 << 20,
        no_proxy: false,
        connect_timeout: Some(Duration::from_secs(10)),
    })
    .map_err(|_| Error::config("Cannot configure the server request"))?;
    crate::flush_stderr(&mut cx.out.stderr);
    allowit_sdk::progress::report("Reading the policy from the Allowit server");
    let unavailable = |status: Option<u16>| {
        let mut error = Error::uncertain(format!(
            "The Allowit server at {origin} did not answer; check ALLOWIT_URL and retry"
        ));
        error.code = allowit_sdk::transport_failure::NETWORK_UNAVAILABLE;
        error.status = status;
        error
    };
    let response = client
        .send_refusal_body(
            Request {
                method: Method::Get,
                url: &url,
                headers: &[("accept".into(), "application/json".into())],
                body: None,
            },
            None,
        )
        .map_err(|_| unavailable(None))?;
    match response.status {
        200..300 => {}
        404 => {
            // A server of an earlier release has no such route; both answer 404.
            let typed = serde_json::from_slice::<Value>(&response.body)
                .ok()
                .is_some_and(|body| body["code"] == "policy_unknown");
            return Err(if typed {
                Error::refused(
                    404,
                    "policy_unknown",
                    format!(
                        "{origin} has no policy with this id; copy the line from the app again"
                    ),
                )
            } else {
                Error::api(
                    404,
                    format!("{origin} does not serve policy help; check ALLOWIT_URL"),
                )
            });
        }
        503 => return Err(unavailable(Some(503))),
        status if status >= 500 => {
            return Err(Error::uncertain(format!("Allowit returned HTTP {status}")));
        }
        status => return Err(Error::api(status, "The policy request was refused")),
    }
    let facts: Facts = serde_json::from_slice(&response.body).map_err(|_| invalid())?;
    Ok((origin, facts))
}

/// One step: a title and its commands.
struct Step {
    title: &'static str,
    note: Option<String>,
    commands: Vec<String>,
}

pub(crate) fn run(args: Args, cx: &mut Ctx) -> Result<i32> {
    // An unset ALLOWIT_URL means the production server, and the printed
    // commands need no variable either.
    let implicit = crate::config::env_value("ALLOWIT_URL").trim().is_empty();
    let (origin, facts) = fetch(cx, &args.id)?;
    let rail = routing::info(&facts.network).map_err(|_| {
        Error::config(format!(
            "this build has no rail {}; install the current allowit",
            facts.network.chars().take(64).collect::<String>()
        ))
    })?;
    let check_account = |key: &Option<String>| -> Result<Option<String>> {
        key.as_ref()
            .map(|key| {
                address::check_account(rail.family, key).map_err(|_| invalid())?;
                shell_safe(key, 128)
            })
            .transpose()
    };
    let server_key = check_account(&facts.server_public_key)?;
    let agent_key = check_account(&facts.agent_public_key)?;
    let endpoint = facts
        .endpoint
        .as_ref()
        .map(|endpoint| {
            if !(endpoint.starts_with("https://") || endpoint.starts_with("http://")) {
                return Err(invalid());
            }
            shell_safe(endpoint, 512)
        })
        .transpose()?;
    let wallet = facts
        .wallet
        .as_ref()
        .map(|wallet| {
            address::check_wallet(rail.family, wallet).map_err(|_| invalid())?;
            shell_safe(wallet, 128)
        })
        .transpose()?;
    let kind = facts
        .wallet_kind
        .as_ref()
        .map(|kind| shell_safe(kind, 32))
        .transpose()?;
    let symbol = facts
        .asset_symbol
        .as_ref()
        .map(|symbol| shell_safe(symbol, 16))
        .transpose()?;
    let limits = facts
        .limits
        .iter()
        .take(64)
        .map(|limit| {
            Ok((
                shown(&limit.key, 64)?,
                shown(&limit.label, 80)?,
                shown(&limit.value, 2000)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let state = match facts.state.as_str() {
        "awaiting_agent_key" | "draft" | "active" | "inactive" => facts.state.as_str(),
        _ => return Err(invalid()),
    };
    // The local key of this policy's agent, if this device holds it.
    let local = match &agent_key {
        Some(agent) => {
            let dir = cx.dir()?;
            keys::list(&dir)?
                .0
                .into_iter()
                .find(|key| key.secret && &key.address == agent)
                .map(|key| key.name)
        }
        None => None,
    };
    // A `--config-dir` flag of this call stays on every printed command: the commands then
    // read and write the directory that holds the key found above. A directory from
    // `ALLOWIT_CONFIG_DIR` stays in the caller's environment.
    let mut cli = match &cx.global.config_dir {
        Some(dir) if cx.global.config_dir_flag => format!(
            "allowit --config-dir {}",
            shell_quote(&dir.to_string_lossy())
        ),
        _ => "allowit".to_owned(),
    };
    // `--rail` or `ALLOWIT_RAIL` of this call outranks the rail that `rail use` saves. When it
    // names another rail than the policy's, every printed command names the policy's rail.
    if let Some(named) = &cx.global.rail
        && routing::info(named).map(|named| named.id).ok() != Some(rail.id)
    {
        cli = format!("{cli} --rail {}", rail.id);
    }
    let with_url = |command: &str| {
        if implicit {
            command.to_owned()
        } else {
            format!("ALLOWIT_URL={origin} {command}")
        }
    };
    let again = with_url(&format!("{cli} {} help", args.id));
    let select = match &endpoint {
        Some(endpoint) => format!("{cli} rail use {} --rpc-url {endpoint}", rail.id),
        None => format!("{cli} rail use {}", rail.id),
    };
    let endpoint_note = endpoint.is_none().then(|| {
        format!(
            "This server names no endpoint. Add --rpc-url URL with a public RPC endpoint of {}.",
            rail.label
        )
    });
    let mut steps = vec![];
    let mut examples = vec![];
    let mut next = None;
    match state {
        "awaiting_agent_key" => {
            // A device that already holds a key named `agent` for this kind of rail uses it:
            // `keys generate` refuses an existing name. A name of another rail family stays
            // untouched, and the new key takes the first unused name.
            let dir = cx.dir()?;
            let (existing, _) = keys::list(&dir)?;
            let reuse = existing
                .iter()
                .find(|key| key.name == "agent" && key.secret && key.check_rail(rail.id).is_ok());
            let name = match reuse {
                Some(key) => key.name.clone(),
                None => (1..)
                    .map(|n| {
                        if n == 1 {
                            "agent".to_owned()
                        } else {
                            format!("agent-{n}")
                        }
                    })
                    .find(|name| !keys::exists(&dir, name))
                    .unwrap_or_else(|| "agent".into()),
            };
            let mut commands = vec![select];
            if reuse.is_none() {
                commands.push(format!("{cli} keys generate {name}"));
            }
            commands.push(format!("{cli} keys use {name}"));
            if rail.testnet {
                commands.push(format!("{cli} keys fund {name}"));
            }
            steps.push(Step {
                title: if reuse.is_some() {
                    "Use your key"
                } else {
                    "Make your key"
                },
                note: endpoint_note,
                commands,
            });
            let public = match reuse {
                Some(key) => format!(
                    "Tell the owner your public key: {}.",
                    shell_safe(&key.address, 128)?
                ),
                None => "Tell the owner the `address` that `keys generate` printed: it is your public key.".to_owned(),
            };
            next = Some(format!(
                "{public} {}Then wait. When the owner has signed, run: {again}",
                if rail.testnet {
                    "`keys fund` creates your account on the test network; it must exist before the owner signs. "
                } else {
                    "Your account must exist and hold enough for network fees before the owner signs. "
                }
            ));
        }
        "draft" => {
            next = Some(match &local {
                Some(name) => format!(
                    "The owner has your public key (local key `{name}`) and has not signed yet. Wait, then run: {again}"
                ),
                None => format!(
                    "The owner entered an agent public key that no local key has. Tell the owner your public key (`{cli} keys show NAME`). Then run: {again}"
                ),
            });
        }
        "active" => {
            let (Some(wallet), Some(server_key), Some(agent)) = (&wallet, &server_key, &agent_key)
            else {
                return Err(invalid());
            };
            if facts.profile.is_none() {
                return Err(invalid());
            }
            match &local {
                None => {
                    next = Some(format!(
                        "No local key has the agent public key {agent} of this policy, so this device cannot pay. Tell the owner your public key (`{cli} keys show NAME`) and stop."
                    ));
                }
                Some(name) => {
                    steps.push(Step {
                        title: "Set up once, in this order",
                        note: endpoint_note,
                        commands: vec![
                            select,
                            format!("{cli} rail trust {origin} {server_key}"),
                            format!("{cli} keys use {name}"),
                            // The attached form: an id can start with a hyphen.
                            with_url(&format!("{cli} wallet import --policy={}", args.id)),
                            format!("{cli} wallet use {wallet}"),
                        ],
                    });
                    // The amount syntax of the installed asset: an issued asset names its
                    // issuer, so the example pays the policy's own asset.
                    let issuer = facts
                        .profile
                        .as_ref()
                        .and_then(|profile| profile["config"]["assetIssuer"].as_str())
                        .filter(|issuer| address::check_account(rail.family, issuer).is_ok())
                        .and_then(|issuer| shell_safe(issuer, 128).ok());
                    let symbol = match (symbol.as_deref(), issuer) {
                        (Some(symbol), Some(issuer)) => format!("{symbol}:{issuer}"),
                        (Some(symbol), None) => symbol.to_owned(),
                        (None, _) => "ASSET".to_owned(),
                    };
                    // The first allowed destination of the policy, when it lists them.
                    let destination = facts
                        .profile
                        .as_ref()
                        .and_then(|profile| {
                            profile["portable"]["params"]["destinations"]["List"][0]["Addr"]
                                .as_str()
                        })
                        .filter(|to| address::check_account(rail.family, to).is_ok())
                        .and_then(|to| shell_safe(to, 128).ok())
                        .unwrap_or_else(|| "DESTINATION".into());
                    // `--wallet` or `ALLOWIT_WALLET` of this call outranks the wallet that
                    // `wallet use` saves. When it names another wallet, the payment names the
                    // policy wallet.
                    let pay = match &cx.global.wallet {
                        Some(named) if named != wallet => format!("{cli} --wallet {wallet}"),
                        _ => cli.clone(),
                    };
                    examples = vec![
                        (
                            "pay",
                            with_url(&format!(
                                "{pay} exec --op transfer --to {destination} --amount {symbol}:AMOUNT --request-id pay-0001"
                            )),
                        ),
                        (
                            "status",
                            with_url(&format!("{cli} status pay-0001 --wait 60s")),
                        ),
                        ("history", format!("{cli} history")),
                    ];
                }
            }
        }
        _ => {
            next = Some(
                "This policy is not active. The owner stopped it or has not finished it. Ask the owner."
                    .into(),
            );
        }
    }
    let rules = [
        "Pay only with `allowit exec`: it pays from the policy wallet. Never pay from your own account; it pays only the network fees.",
        "Exit 20 means the policy denied the request. Do not retry, split or work around it. Tell the owner the rule that the error names.",
        "Never print, copy or move a secret key. Never change the policy: only the owner can.",
    ];
    if cx.out.json {
        let value = json!({
            "policyId": args.id,
            "server": origin,
            "state": state,
            "network": rail.id,
            "networkLabel": rail.label,
            "endpoint": endpoint,
            "walletKind": kind,
            "wallet": wallet,
            "agentPublicKey": agent_key,
            "localKey": local,
            "serverPublicKey": server_key,
            "steps": steps.iter().map(|step| json!({"title": step.title, "note": step.note, "commands": step.commands})).collect::<Vec<_>>(),
            "examples": examples.iter().map(|(name, command)| (name.to_string(), json!(command))).collect::<serde_json::Map<_, _>>(),
            "limits": limits.iter().map(|(key, label, value)| json!({"key": key, "label": label, "value": value})).collect::<Vec<_>>(),
            "next": next,
            "rules": rules,
            "guide": "allowit guide",
        });
        cx.out.json_value(&value);
        return Ok(0);
    }
    let out = &mut cx.out;
    out.line(format!(
        "Allowit policy {} on {origin}: {}",
        args.id,
        state.replace('_', " ")
    ));
    let mut rows = vec![(
        "network".to_owned(),
        format!("{} ({})", rail.label, rail.id),
    )];
    if let Some(wallet) = &wallet {
        let kind = kind
            .as_deref()
            .map(|kind| format!(" ({kind})"))
            .unwrap_or_default();
        rows.push(("wallet".into(), format!("{wallet}{kind}")));
    } else if let Some(kind) = &kind {
        rows.push(("wallet kind".into(), kind.clone()));
    }
    if let Some(agent) = &agent_key {
        let held = match &local {
            Some(name) => format!("  local key: {name}"),
            None => "  no local key".into(),
        };
        rows.push(("agent key".into(), format!("{agent}{held}")));
    }
    if let Some(server_key) = &server_key {
        rows.push(("server key".into(), server_key.clone()));
    }
    out.stdout.push_str(&crate::term::render(&rows));
    if server_key.is_some() {
        out.line(
            "The owner sees the server key in the app under Agent access. If the two differ, stop.",
        );
    }
    for step in &steps {
        out.line("");
        out.line(format!("{}:", step.title));
        for command in &step.commands {
            out.line(format!("  {command}"));
        }
        if let Some(note) = &step.note {
            out.line(note);
        }
    }
    if !examples.is_empty() {
        out.line("");
        out.line("Pay, check the result and list your requests:");
        for (_, command) in &examples {
            out.line(format!("  {command}"));
        }
    }
    if !limits.is_empty() {
        out.line("");
        out.line("Limits:");
        let rows: Vec<_> = limits
            .iter()
            .map(|(_, label, value)| (format!("  {label}"), value.clone()))
            .collect();
        out.stdout.push_str(&crate::term::render(&rows));
    }
    if let Some(next) = &next {
        out.line("");
        out.line(next);
    }
    out.line("");
    out.line("Rules:");
    for rule in rules {
        out.line(format!("  {rule}"));
    }
    out.line("More: allowit guide");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_values_enter_a_command_line_only_without_shell_characters() {
        for safe in [
            "https://rpc.example/v1",
            "ACCOUNT0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            "example:testnet",
        ] {
            assert_eq!(shell_safe(safe, 512).unwrap(), safe);
        }
        for hostile in [
            "https://rpc.example/;rm -rf ~",
            "https://rpc.example/$(id)",
            "a`b`",
            "a b",
            "a\nb",
            "a|b",
            "a&b",
            "a>b",
            "a'b",
            "a\"b",
            "a\\b",
            "a#b",
            "a*b",
            "a?b",
            "a=b",
            "",
        ] {
            assert!(shell_safe(hostile, 512).is_err(), "{hostile:?}");
        }
        assert!(shell_safe(&"a".repeat(513), 512).is_err());
        assert!(shown("line\nbreak", 80).is_err());
        assert!(shown("\u{1b}[31m", 80).is_err());
        assert_eq!(shown("1 UNIT", 80).unwrap(), "1 UNIT");
    }

    #[test]
    fn a_local_path_is_one_shell_word() {
        assert_eq!(
            shell_quote("/home/agent/.config/allowit"),
            "/home/agent/.config/allowit"
        );
        assert_eq!(shell_quote("/tmp/my dir"), "'/tmp/my dir'");
        assert_eq!(shell_quote("/tmp/it's"), "'/tmp/it'\\''s'");
        assert_eq!(shell_quote("$(id)"), "'$(id)'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn a_policy_id_is_a_bounded_url_safe_word() {
        for id in [
            "Xg7IxRXCr2MhVdRrTFoYKeAw2iV4u463",
            "-1WagPbql_eK8Cwz",
            "abcdefgh",
        ] {
            assert!(is_policy_id(id), "{id}");
        }
        for other in [
            "short",
            "has space1",
            "semi;colon1",
            "slash/slash",
            &"x".repeat(129),
        ] {
            assert!(!is_policy_id(other), "{other}");
        }
    }
}
