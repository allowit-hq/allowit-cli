//! `allowit POLICY_ID help`: what the agent of one policy does next.
//!
//! The owner copies one line from the app. It holds the policy id and nothing
//! else. The id is a pointer, not a credential: by the id alone the server
//! answers only the public facts that an agent needs before it proves its key
//! (state, network and endpoint, wallet kind, server key). This command reads
//! them, then asks as the policy's agent with each local key of the rail, the
//! selected one first, until the server admits one: only the agent reads its
//! wallet, limits, profile and the owner's words. It prints the agent's next
//! commands in order, with every value filled in, and the owner's instructions
//! as the owner wrote them. It changes nothing: it pins no server key and
//! imports no profile. The server key that it prints is the server's own claim;
//! the owner compares it with the key that the app shows, and `wallet import`
//! pins it.
use crate::{
    agent_read,
    ctx::Ctx,
    error::{Error, Result},
    rails::address,
    routing::{self, RailInfo},
    settings::keys,
};
use allowit_sdk::{
    transport::{
        http::{HttpClient, Method, Request},
        rpc::Options,
    },
    wallet::LocalSigner,
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

/// The server's answer to `GET /api/agent/policy?id=POLICY_ID`, the public
/// facts, or to the agent's signed `POST /api/agent/policy`, which adds the
/// agent key, the owner's words, the wallet, the limits, the profile and the
/// credits. Unknown fields are ignored, so a later server can add facts.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Facts {
    #[serde(default)]
    pub policy_id: Option<String>,
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
    #[serde(default)]
    pub credits: Option<CreditFacts>,
    /// Each message that the owner wrote when making the policy, oldest first.
    #[serde(default)]
    pub owner_words: Vec<String>,
}

/// What an evaluation of the policy costs its owner, and whether the owner
/// can pay one now.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreditFacts {
    pub evaluation: String,
    pub tariff: String,
    #[serde(default)]
    pub owner_can_pay: Option<bool>,
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

/// The most owner messages that the help shows: a policy's conversation holds fewer.
const MAX_OWNER_MESSAGES: usize = 32;
/// The most characters of one owner message: the server's bound of a policy request.
const MAX_OWNER_MESSAGE: usize = 3_000;

/// The owner's words as the help prints them: each message as the owner wrote it, with line
/// breaks kept and no other control character, so a message cannot move the cursor, color or
/// clear the terminal. An empty message is left out. More messages or a longer one than the
/// server allows are invalid facts.
fn owner_words(words: &[String]) -> Result<Vec<String>> {
    if words.len() > MAX_OWNER_MESSAGES {
        return Err(invalid());
    }
    words
        .iter()
        .map(|text| {
            if text.chars().count() > MAX_OWNER_MESSAGE {
                return Err(invalid());
            }
            let text = text.replace("\r\n", "\n").replace('\r', "\n");
            Ok(text
                .chars()
                .map(|c| if c == '\t' { ' ' } else { c })
                .filter(|c| *c == '\n' || !c.is_control())
                .collect::<String>()
                .trim()
                .to_owned())
        })
        .filter(|text| !matches!(text, Ok(text) if text.is_empty()))
        .collect()
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
        // Inside single quotes a shell reads every character as itself.
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

fn invalid() -> Error {
    Error::uncertain("The server returned invalid policy facts")
}

/// The server at `origin` did not answer.
fn unavailable(origin: &str, status: Option<u16>) -> Error {
    let mut error = Error::uncertain(format!(
        "The Allowit server at {origin} did not answer; check ALLOWIT_URL and retry"
    ));
    error.code = allowit_sdk::transport_failure::NETWORK_UNAVAILABLE;
    error.status = status;
    error
}

/// The refusal of a policy id that the server at `origin` does not hold. A server of an
/// earlier release has no such route; both answer 404.
fn unknown(origin: &str, body: &[u8], route: &str) -> Error {
    let typed = serde_json::from_slice::<Value>(body)
        .ok()
        .is_some_and(|body| body["code"] == "policy_unknown");
    if typed {
        Error::refused(
            404,
            "policy_unknown",
            format!("{origin} has no policy with this id; copy the line from the app again"),
        )
    } else {
        Error::api(
            404,
            format!("{origin} does not serve {route}; check ALLOWIT_URL"),
        )
    }
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
        .map_err(|_| unavailable(&origin, None))?;
    match response.status {
        200..300 => {}
        404 => return Err(unknown(&origin, &response.body, "policy help")),
        503 => return Err(unavailable(&origin, Some(503))),
        status if status >= 500 => {
            return Err(Error::uncertain(format!("Allowit returned HTTP {status}")));
        }
        status => return Err(Error::api(status, "The policy request was refused")),
    }
    let facts: Facts = serde_json::from_slice(&response.body).map_err(|_| invalid())?;
    if facts.policy_id.as_deref().is_some_and(|named| named != id) {
        return Err(invalid());
    }
    Ok((origin, facts))
}

/// The facts of `id` that the server at `origin` answers to the policy's agent, read with the
/// request proof of `signer`. `None` when the server refuses the key as not the policy's agent
/// key: such a key reads nothing more than the public facts.
pub(crate) fn read_as_agent(origin: &str, id: &str, signer: &LocalSigner) -> Result<Option<Facts>> {
    let (status, body) = agent_read::post(
        origin,
        agent_read::POLICY_PATH,
        &json!({ "policyId": id }),
        signer,
        agent_read::Limits {
            timeout: Duration::from_secs(20),
            connect: Duration::from_secs(10),
            max_bytes: 1 << 20,
        },
    )
    .map_err(|_| unavailable(origin, None))?;
    match status {
        200..300 => {}
        _ if agent_read::not_the_agent(status, &body) => return Ok(None),
        401 => {
            return Err(Error::refused(
                401,
                "request_proof_refused",
                format!(
                    "{origin} refused the signature of key {}; check that this device's clock is right, then run the command again",
                    signer.address()
                ),
            ));
        }
        404 => return Err(unknown(origin, &body, "the agent's policy read")),
        // A server of an earlier release reads the policy with GET only.
        405 => {
            return Err(Error::api(
                405,
                format!("{origin} does not serve the agent's policy read; check ALLOWIT_URL"),
            ));
        }
        503 => return Err(unavailable(origin, Some(503))),
        status if status >= 500 => {
            return Err(Error::uncertain(format!("Allowit returned HTTP {status}")));
        }
        status => return Err(Error::api(status, "The policy request was refused")),
    }
    let facts: Facts = serde_json::from_slice(&body).map_err(|_| invalid())?;
    if facts.policy_id.as_deref() != Some(id)
        || facts.agent_public_key.as_deref() != Some(signer.address())
    {
        return Err(invalid());
    }
    Ok(Some(facts))
}

/// The most local keys that the help tries as the policy's agent.
const MAX_AGENT_KEYS: usize = 8;

/// What the search for the local agent key of a policy found.
enum LocalAgent {
    /// The local key that the server admits as the policy's agent, with its facts.
    Found(String, Box<Facts>),
    /// The server admits none of the local keys of the rail.
    None,
    /// The server admits none of the keys that the search tried, and more keys were left.
    Untried,
}

/// The local key of `rail` that is the agent key of policy `id`, with the facts that only the
/// agent reads. The rail's selected agent key goes first, then each other local key of the
/// rail; the first key that the server admits ends the search. At most [`MAX_AGENT_KEYS`] keys
/// are tried; a search that leaves keys untried says so.
fn local_agent(cx: &mut Ctx, rail: &RailInfo, origin: &str, id: &str) -> Result<LocalAgent> {
    let dir = cx.dir()?;
    let selected = cx.settings()?.rail(rail.id).agent;
    let mut candidates: Vec<keys::Key> = keys::list(&dir)?
        .0
        .into_iter()
        .filter(|key| key.secret && key.check_rail(rail.id).is_ok())
        .collect();
    candidates.sort_by_key(|key| Some(&key.name) != selected.as_ref());
    let untried = candidates.len() > MAX_AGENT_KEYS;
    for key in candidates.into_iter().take(MAX_AGENT_KEYS) {
        let signer = allowit_sdk::wallet::local_signer(rail.id, &keys::signer_path(&dir, &key)?)?;
        if let Some(facts) = read_as_agent(origin, id, &signer)? {
            return Ok(LocalAgent::Found(key.name, Box::new(facts)));
        }
    }
    Ok(if untried {
        LocalAgent::Untried
    } else {
        LocalAgent::None
    })
}

/// The server key that this command prints is the server's own claim. Text and JSON output
/// both carry this sentence before the setup commands; `wallet import` pins the key.
const SERVER_KEY_CHECK: &str =
    "The owner sees the server key in the app under Agent access. If the two differ, stop.";

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
    let state = match facts.state.as_str() {
        "awaiting_agent_key" | "draft" | "active" | "inactive" => facts.state.clone(),
        _ => return Err(invalid()),
    };
    let state = state.as_str();
    // Only the policy's agent reads its key, wallet, limits and profile. A device that holds
    // no local key that the server admits as the agent reads the public facts only.
    let (local, mine, untried) = match state {
        "draft" | "active" => match local_agent(cx, rail, &origin, &args.id)? {
            LocalAgent::Found(name, mine) => (Some(name), Some(*mine), false),
            LocalAgent::None => (None, None, false),
            LocalAgent::Untried => (None, None, true),
        },
        _ => (None, None, false),
    };
    // The agent's answer holds the public facts too, and they must be the same.
    if let Some(mine) = &mine
        && (mine.state != facts.state
            || mine.network != facts.network
            || mine.server_public_key != facts.server_public_key)
    {
        return Err(invalid());
    }
    let mine = mine.as_ref();
    let check_account = |key: Option<&String>| -> Result<Option<String>> {
        key.map(|key| {
            address::check_account(rail.family, key).map_err(|_| invalid())?;
            shell_safe(key, 128)
        })
        .transpose()
    };
    let server_key = check_account(facts.server_public_key.as_ref())?;
    let agent_key = check_account(mine.and_then(|mine| mine.agent_public_key.as_ref()))?;
    let endpoint = facts
        .endpoint
        .as_ref()
        .map(|endpoint| {
            if !(endpoint.starts_with("https://") || endpoint.starts_with("http://")) {
                return Err(invalid());
            }
            // An IPv6 host has brackets, which a shell reads as a pattern: the URL without
            // them is plain, and the printed command quotes the whole URL.
            let plain: String = endpoint
                .chars()
                .filter(|c| !matches!(c, '[' | ']'))
                .collect();
            shell_safe(&plain, 512)?;
            Ok(endpoint.clone())
        })
        .transpose()?;
    let wallet = mine
        .and_then(|mine| mine.wallet.as_ref())
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
    let symbol = mine
        .and_then(|mine| mine.asset_symbol.as_ref())
        .map(|symbol| shell_safe(symbol, 16))
        .transpose()?;
    let limits = mine
        .map(|mine| mine.limits.as_slice())
        .unwrap_or_default()
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
    // What the owner told the agent when making the policy. Only the policy's agent reads it.
    let words = owner_words(
        mine.map(|mine| mine.owner_words.as_slice())
            .unwrap_or_default(),
    )?;
    // The price of one evaluation, which the owner pays. Only the policy's agent reads it.
    let credits = mine
        .and_then(|mine| mine.credits.as_ref())
        .map(|credits| {
            let plain = crate::credits::Charge::of(&json!({
                "charged": credits.evaluation, "tariff": credits.tariff, "lines": [],
            }))
            .ok_or_else(invalid)?;
            Ok((plain.charged, plain.tariff, credits.owner_can_pay))
        })
        .transpose()?;
    let credits_note = credits.as_ref().map(|(evaluation, _, can_pay)| {
        let unpaid = if *can_pay == Some(false) {
            " The owner has no credits left now: Allowit refuses each request with `credits_exhausted` until the owner gets credits. Tell the owner."
        } else {
            ""
        };
        format!(
            "Credits: each request that you sign costs the policy owner {evaluation} credits, whatever its outcome, and the model tokens that it uses, in whole credits. You need no credits of your own.{unpaid}"
        )
    });
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
            // An IPv6 origin has brackets: the value is one quoted shell word.
            format!("ALLOWIT_URL={} {command}", shell_quote(&origin))
        }
    };
    let again = with_url(&format!("{cli} {} help", args.id));
    let select = match &endpoint {
        Some(endpoint) => format!(
            "{cli} rail use {} --rpc-url {}",
            rail.id,
            shell_quote(endpoint)
        ),
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
                "{public} Then wait. When the owner has signed, run: {again}"
            ));
        }
        "draft" | "active" if untried => {
            next = Some(format!(
                "This device holds more than {MAX_AGENT_KEYS} keys of this rail, and none that was tried is the agent key of this policy. Select the key that you gave the owner (`{cli} keys use NAME`), then run: {again}"
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
        "active" => match (&local, mine) {
            (Some(name), Some(mine)) => {
                let (Some(wallet), Some(_), Some(_)) = (&wallet, &server_key, &agent_key) else {
                    return Err(invalid());
                };
                if mine.profile.is_none() {
                    return Err(invalid());
                }
                steps.push(Step {
                    title: "Set up once, in this order",
                    note: endpoint_note,
                    commands: vec![
                        select,
                        format!("{cli} keys use {name}"),
                        // The attached form: an id can start with a hyphen.
                        with_url(&format!("{cli} wallet import --policy={}", args.id)),
                        format!("{cli} wallet use {wallet}"),
                    ],
                });
                // The amount syntax of the installed asset: an issued asset names its
                // issuer, so the example pays the policy's own asset.
                let issuer = mine
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
                let destination = mine
                    .profile
                    .as_ref()
                    .and_then(|profile| {
                        profile["portable"]["params"]["destinations"]["List"][0]["Addr"].as_str()
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
            _ => {
                next = Some(format!(
                    "No local key is the agent key of this policy, so this device cannot pay. Tell the owner your public key (`{cli} keys show NAME`) and stop."
                ));
            }
        },
        _ => {
            next = Some(
                "This policy is not active. The owner stopped it or has not finished it. Ask the owner."
                    .into(),
            );
        }
    }
    let rules = [
        "Start when the owner tells you to, or at once when the owner's instructions already say what to do. Setting up is not a task, and a printed payment is an example: run no test payment.",
        "When a request disagrees with the owner's instructions, ask the owner before you pay. Never pay outside the policy.",
        "Pay only with `allowit exec`: it pays from the policy wallet.",
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
            "serverKeyCheck": server_key.as_ref().map(|_| SERVER_KEY_CHECK),
            "steps": steps.iter().map(|step| json!({"title": step.title, "note": step.note, "commands": step.commands})).collect::<Vec<_>>(),
            "examples": examples.iter().map(|(name, command)| (name.to_string(), json!(command))).collect::<serde_json::Map<_, _>>(),
            "ownerWords": words,
            "limits": limits.iter().map(|(key, label, value)| json!({"key": key, "label": label, "value": value})).collect::<Vec<_>>(),
            "credits": credits.as_ref().map(|(evaluation, tariff, can_pay)| json!({
                "evaluation": evaluation, "tariff": tariff, "paidBy": "owner", "ownerCanPay": can_pay,
            })),
            "creditsNote": credits_note,
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
    if let Some((evaluation, tariff, _)) = &credits {
        rows.push((
            "credits".into(),
            format!("{evaluation} per request, charged to the owner (price list {tariff})"),
        ));
    }
    out.stdout.push_str(&crate::term::render(&rows));
    if server_key.is_some() {
        out.line(SERVER_KEY_CHECK);
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
        out.line("To pay (examples, not requests):");
        for (_, command) in &examples {
            out.line(format!("  {command}"));
        }
    }
    if !words.is_empty() {
        out.line("");
        out.line("Owner's instructions (as the owner wrote them when making this policy):");
        for text in &words {
            let mut lines = text.lines();
            out.line(format!("  - {}", lines.next().unwrap_or_default()));
            for line in lines {
                out.line(format!("    {line}").trim_end());
            }
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
    if let Some(note) = &credits_note {
        out.line("");
        out.line(note);
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
    fn the_owner_words_keep_their_text_and_lines_and_lose_control_characters() {
        let words = [
            "Every Friday pay Ana 100.\r\nOnly Ana.".to_owned(),
            "   ".to_owned(),
            "Red \u{1b}[31mtext\u{7}\tand a tab".to_owned(),
        ];
        assert_eq!(
            owner_words(&words).unwrap(),
            [
                "Every Friday pay Ana 100.\nOnly Ana.",
                "Red [31mtext and a tab"
            ]
        );
        assert!(owner_words(&["x".repeat(MAX_OWNER_MESSAGE)]).is_ok());
        assert!(owner_words(&["x".repeat(MAX_OWNER_MESSAGE + 1)]).is_err());
        assert!(owner_words(&vec!["x".to_owned(); MAX_OWNER_MESSAGES + 1]).is_err());
        assert!(owner_words(&[]).unwrap().is_empty());
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
