//! What the agent's CLI checks before the agent key signs anything for a policy.
//!
//! The server key: the policy's server key comes from the server's public answer and is pinned
//! for that server at `wallet import --policy`. A pinned key never changes from a server
//! answer. Before a signature, the profile's server key, which is the key that co-signs every
//! operation, and the server's current answer must both be the pinned key.
//!
//! The wallet: the policy, as the server answers it to its agent, must be active and name the
//! wallet of the profile, and the ledger must hold that wallet, unpaused. What the ledger must
//! show of the agent depends on the wallet kind. A Native wallet is an account that the owner
//! owns: the agent has a key and no account of its own, and the agent key and the server key
//! must be signers of the wallet account. A contract wallet (Simple, Standard, Custom) is a
//! contract: its agent has neither an account nor a signer seat, and the contract checks the
//! agent's signature itself, so the ledger must hold the contract and nothing of the agent.
use crate::{
    commands::policy_help::{self, Facts},
    ctx::Ctx,
    error::{Error, Result},
    routing::{self, RailInfo},
};
use allowit_sdk::{
    config::ServerAuthority,
    kinds::WalletKind,
    profiles::{Description, Profile},
    wallet::LocalSigner,
};

/// A refusal before any signature: the CLI exits 4 with `code`, and nothing was signed.
fn refused(code: &str, message: String) -> Error {
    let mut error = Error::config(message);
    error.code = 4;
    error.config = false;
    error.api_code = Some(code.into());
    error
}

/// The key that the server names for a policy differs from the key that this device pinned.
pub(crate) const SERVER_KEY_DIFFERS: &str = "server_key_differs";

/// A server key that `wallet import --policy` pins for a server once its import stands.
pub(crate) struct Pin {
    pub authority: ServerAuthority,
    /// True when the server had no pinned key before this import.
    pub new: bool,
}
impl Pin {
    /// Saves the key for its server on `rail`, unless one is pinned. A pinned key is never
    /// replaced from a server answer: only `rail trust` changes it.
    pub(crate) fn save(&self, cx: &mut Ctx, rail: &RailInfo) -> Result<()> {
        if !self.new {
            return Ok(());
        }
        let (origin, key) = (self.authority.origin(), self.authority.public_key());
        cx.settings()?.update(|config| {
            config
                .rail_mut(rail.id)
                .server_authorities
                .entry(origin.into())
                .or_insert_with(|| key.into());
            Ok(())
        })
    }
}

/// The profile that the server answers to the policy's agent, with the server key to pin.
pub(crate) struct PolicyProfile {
    pub profile: Vec<u8>,
    pub pin: Option<Pin>,
}

/// The public profile of policy `id` from the `ALLOWIT_URL` server, read as its agent with the
/// selected agent key of `rail`. The server key comes from the public answer: it must be the
/// key that this device pinned for that server, if it pinned one.
pub(crate) fn policy_profile(cx: &mut Ctx, rail: &RailInfo, id: &str) -> Result<PolicyProfile> {
    let (origin, facts) = policy_help::fetch(cx, id)?;
    let network = routing::info(&facts.network)
        .map_err(|_| Error::uncertain("The server returned invalid policy facts"))?;
    if network.id != rail.id {
        return Err(Error::config(format!(
            "this policy is on {}; run allowit rail use {} first",
            network.id, network.id
        )));
    }
    if facts.state != "active" {
        return Err(Error::config(format!(
            "this policy is not active yet; run allowit {id} help"
        )));
    }
    let key = facts
        .server_public_key
        .clone()
        .ok_or_else(|| Error::uncertain("The server returned invalid policy facts"))?;
    let authority = ServerAuthority::new(rail.id, &origin, &key)
        .map_err(|_| Error::uncertain("The server returned invalid policy facts"))?;
    let pinned = cx
        .settings()?
        .rail(rail.id)
        .server_authorities
        .get(authority.origin())
        .cloned();
    if let Some(pinned) = &pinned
        && pinned != authority.public_key()
    {
        return Err(refused(
            SERVER_KEY_DIFFERS,
            format!(
                "{origin} names server key {key} for this policy, and this device trusts {pinned} for that server. Nothing was imported. Ask the owner which server key the app shows under Agent access."
            ),
        ));
    }
    let agent = cx.agent(rail)?;
    let signer = allowit_sdk::wallet::local_signer(rail.id, &cx.signer(&agent)?)?;
    let mine = policy_help::read_as_agent(&origin, id, &signer)?.ok_or_else(|| {
        refused(
            crate::agent_read::NOT_POLICY_AGENT,
            format!(
                "the selected key {} ({}) is not the agent key of this policy. Select the key that you gave the owner with allowit keys use NAME, or tell the owner this public key.",
                agent.name, agent.address
            ),
        )
    })?;
    if mine.state != facts.state || mine.server_public_key.as_deref() != Some(key.as_str()) {
        return Err(Error::uncertain("The server returned invalid policy facts"));
    }
    let profile = mine.profile.ok_or_else(|| {
        Error::config(format!(
            "this policy is not active yet; run allowit {id} help"
        ))
    })?;
    // The answer must be the profile of the policy that was asked for: the checks of the
    // import prove the server and the chain state of a profile, not which policy it is.
    if profile["policyId"].as_str() != Some(id) {
        return Err(Error::uncertain(
            "The server returned the profile of another policy",
        ));
    }
    let profile =
        serde_json::to_vec(&profile).map_err(|_| Error::config("Cannot encode the profile"))?;
    let (description, _) =
        allowit_sdk::plugins::for_network(rail.id)?.decode_profile(rail.id, &profile)?;
    if description.server != key {
        return Err(refused(
            SERVER_KEY_DIFFERS,
            format!(
                "the profile of this policy names server key {}, and {origin} names {key}. Nothing was imported. Ask the owner.",
                description.server
            ),
        ));
    }
    if description.agent != signer.address() {
        return Err(Error::uncertain(
            "The server returned the profile of another agent",
        ));
    }
    Ok(PolicyProfile {
        profile,
        pin: Some(Pin {
            authority,
            new: pinned.is_none(),
        }),
    })
}

/// The server authority of `profile`: the server key pinned for its server on `rail`. The
/// profile's server key, which co-signs every operation, must be that key.
pub(crate) fn authority(
    cx: &mut Ctx,
    rail: &RailInfo,
    profile: &Profile,
) -> Result<ServerAuthority> {
    let origin = profile.server_origin().to_owned();
    let named = crate::config::parse_origin(&cx.server_url()?)?;
    let d = profile.description();
    let pinned = cx
        .settings()?
        .rail(rail.id)
        .server_authorities
        .get(&named)
        .cloned()
        .ok_or_else(|| {
            Error::config(format!(
                "no trusted server key for {named} on {}; run allowit wallet import --policy={}, or allowit rail trust {named} PUBLIC_KEY",
                rail.id, d.policy_id
            ))
        })?;
    if named != origin {
        return Err(Error::config(format!(
            "the policy {} of this wallet is on {origin}, and ALLOWIT_URL names {named}; run the command with ALLOWIT_URL={origin}",
            d.policy_id
        )));
    }
    server_key_pinned(d, &origin, &pinned)?;
    ServerAuthority::new(rail.id, &origin, &pinned)
}

/// The profile's server key is the key pinned for its server.
fn server_key_pinned(d: &Description, origin: &str, pinned: &str) -> Result<()> {
    if d.server != pinned {
        return Err(refused(
            SERVER_KEY_DIFFERS,
            format!(
                "the policy {} of this wallet names server key {}, and the key pinned for {origin} is {pinned}. Nothing was signed. Ask the owner which server key the app shows under Agent access.",
                d.policy_id, d.server
            ),
        ));
    }
    Ok(())
}

/// Before the agent signs a fresh operation of `profile`: the policy, read as its agent, is
/// active and names this wallet and the pinned server key, and the ledger holds the wallet as
/// its kind needs. `pinned` is the server key that [`authority`] returned.
pub(crate) fn check_before_signing(
    rail: &RailInfo,
    profile: &Profile,
    pinned: &str,
    signer: &LocalSigner,
    rpc: Option<&str>,
) -> Result<()> {
    let d = profile.description();
    let origin = profile.server_origin();
    let mine = policy_help::read_as_agent(origin, &d.policy_id, signer)?;
    judge_policy(d, origin, pinned, signer.address(), mine.as_ref())?;
    let rpc = rpc.filter(|rpc| !rpc.is_empty()).ok_or_else(|| {
        Error::config(format!(
            "select an RPC endpoint to check the wallet before signing: allowit rail use {} --rpc-url URL",
            rail.id
        ))
    })?;
    let document = profile.document()?;
    let bytes =
        serde_json::to_vec(&document).map_err(|_| Error::config("Cannot encode the profile"))?;
    let kind = allowit_sdk::kinds::wallet_kind(rail.id, &bytes)?;
    let ledger = read_ledger(rail, &document, d, rpc);
    judge_ledger(rail, kind, d, ledger)
}

/// What the ledger shows of a wallet: whether it is paused, and the signer weight of the agent
/// key and of the server key on the wallet account. A contract wallet has no such weights.
struct Ledger {
    paused: bool,
    agent_weight: u32,
    server_weight: u32,
}

/// The wallet of `document` as the ledger shows it through `rpc`. The error is the cause that
/// the rail reports, such as a wallet that the ledger does not hold.
fn read_ledger(
    rail: &RailInfo,
    document: &serde_json::Value,
    d: &Description,
    rpc: &str,
) -> std::result::Result<Ledger, String> {
    let read = || -> Result<Ledger> {
        let config = serde_json::to_vec(&document["config"])
            .map_err(|_| Error::config("Cannot encode the wallet configuration"))?;
        let configuration = allowit_sdk::runtime::owner::configuration(rail.id, &config)?;
        let snapshot = allowit_sdk::runtime::Runtime::new(rail.id, rpc)?
            .discover_snapshot(&configuration, None)?;
        Ok(Ledger {
            paused: snapshot.paused,
            agent_weight: snapshot.signer_weight(&d.agent).unwrap_or(0),
            server_weight: snapshot.signer_weight(&d.server).unwrap_or(0),
        })
    };
    read().map_err(|error| error.message)
}

/// The policy as the server answers it to its agent, before a signature.
fn judge_policy(
    d: &Description,
    origin: &str,
    pinned: &str,
    agent: &str,
    mine: Option<&Facts>,
) -> Result<()> {
    let id = &d.policy_id;
    let Some(mine) = mine else {
        return Err(refused(
            crate::agent_read::NOT_POLICY_AGENT,
            format!(
                "{origin} does not admit key {agent} as the agent of policy {id} now. Nothing was signed. Ask the owner."
            ),
        ));
    };
    if mine.state != "active" {
        return Err(refused(
            "policy_not_active",
            format!(
                "policy {id} is not active now: the owner stopped or changed it. Nothing was signed. Run allowit {id} help, or ask the owner."
            ),
        ));
    }
    if mine.server_public_key.as_deref() != Some(pinned) {
        return Err(refused(
            SERVER_KEY_DIFFERS,
            format!(
                "{origin} now names server key {} for policy {id}, and the key pinned at wallet import is {pinned}. Nothing was signed. Ask the owner which server key the app shows under Agent access.",
                mine.server_public_key.as_deref().unwrap_or("none")
            ),
        ));
    }
    match mine.wallet.as_deref() {
        Some(wallet) if wallet == d.wallet => Ok(()),
        named => Err(refused(
            "wallet_differs",
            format!(
                "policy {id} now pays from wallet {}, not from {}. Nothing was signed. Run allowit wallet import --policy={id}, then allowit wallet use with the new wallet.",
                named.unwrap_or("none"),
                d.wallet
            ),
        )),
    }
}

/// The wallet on the ledger, before a signature, as its kind needs it.
fn judge_ledger(
    rail: &RailInfo,
    kind: WalletKind,
    d: &Description,
    ledger: std::result::Result<Ledger, String>,
) -> Result<()> {
    let wallet = &d.wallet;
    let ledger = ledger.map_err(|cause| {
        refused(
            "wallet_unreadable",
            format!(
                "wallet {wallet} of policy {} is not readable on {}: {cause}. Nothing was signed. If the endpoint answers (allowit rail show), ask the owner whether the wallet still exists.",
                d.policy_id, rail.label
            ),
        )
    })?;
    if ledger.paused {
        return Err(refused(
            "wallet_paused",
            format!("the owner paused wallet {wallet}. Nothing was signed. Ask the owner."),
        ));
    }
    if kind == WalletKind::Native {
        if ledger.agent_weight == 0 {
            return Err(refused(
                "agent_not_signer",
                format!(
                    "your key {} is not a signer of wallet account {wallet}. Nothing was signed. Ask the owner.",
                    d.agent
                ),
            ));
        }
        if ledger.server_weight == 0 {
            return Err(refused(
                "server_not_signer",
                format!(
                    "the server key {} is not a signer of wallet account {wallet}, so the server cannot co-sign. Nothing was signed. Ask the owner.",
                    d.server
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn description() -> Description {
        Description {
            network: "example:testnet".into(),
            wallet: "WALLET".into(),
            agent: "AGENT".into(),
            policy_id: "Xg7IxRXCr2MhVdRrTFoYKeAw".into(),
            owner: "OWNER".into(),
            server: "SERVER".into(),
            asset: "ASSET".into(),
            asset_symbol: "UNIT".into(),
            source: String::new(),
            source_hash: String::new(),
            module_digest: String::new(),
            params: BTreeMap::new(),
        }
    }
    fn facts(state: &str, wallet: &str, server: &str) -> Facts {
        serde_json::from_value(serde_json::json!({
            "policyId": "Xg7IxRXCr2MhVdRrTFoYKeAw", "state": state, "network": "example:testnet",
            "serverPublicKey": server, "agentPublicKey": "AGENT", "wallet": wallet,
        }))
        .unwrap()
    }
    fn code(result: Result<()>) -> Option<String> {
        let error = result.unwrap_err();
        assert_eq!(error.code, 4, "{}", error.message);
        assert!(
            error.message.contains("Nothing was signed"),
            "{}",
            error.message
        );
        error.api_code
    }
    fn rail() -> &'static RailInfo {
        routing::all().first().expect("a registered rail")
    }
    fn ledger(paused: bool, agent: u32, server: u32) -> std::result::Result<Ledger, String> {
        Ok(Ledger {
            paused,
            agent_weight: agent,
            server_weight: server,
        })
    }

    #[test]
    fn a_server_key_other_than_the_pinned_one_is_refused_before_signing() {
        let d = description();
        assert!(server_key_pinned(&d, "https://server.example", "SERVER").is_ok());
        assert_eq!(
            code(server_key_pinned(&d, "https://server.example", "OTHER")).as_deref(),
            Some(SERVER_KEY_DIFFERS)
        );
        // The server's current answer names another key than the pinned one.
        let now = facts("active", "WALLET", "OTHER");
        assert_eq!(
            code(judge_policy(&d, "https://s", "SERVER", "AGENT", Some(&now))).as_deref(),
            Some(SERVER_KEY_DIFFERS)
        );
    }

    #[test]
    fn the_policy_must_be_active_and_name_this_wallet_for_this_agent() {
        let d = description();
        let ok = facts("active", "WALLET", "SERVER");
        assert!(judge_policy(&d, "https://s", "SERVER", "AGENT", Some(&ok)).is_ok());
        for (state, code_name) in [
            ("inactive", "policy_not_active"),
            ("draft", "policy_not_active"),
        ] {
            let now = facts(state, "WALLET", "SERVER");
            assert_eq!(
                code(judge_policy(&d, "https://s", "SERVER", "AGENT", Some(&now))).as_deref(),
                Some(code_name)
            );
        }
        let moved = facts("active", "ANOTHER", "SERVER");
        assert_eq!(
            code(judge_policy(
                &d,
                "https://s",
                "SERVER",
                "AGENT",
                Some(&moved)
            ))
            .as_deref(),
            Some("wallet_differs")
        );
        // The server no longer admits this key as the policy's agent.
        assert_eq!(
            code(judge_policy(&d, "https://s", "SERVER", "AGENT", None)).as_deref(),
            Some(crate::agent_read::NOT_POLICY_AGENT)
        );
    }

    #[test]
    fn the_ledger_must_hold_the_wallet_as_its_kind_needs() {
        let d = description();
        // A wallet that the ledger does not hold, or that it cannot read.
        assert_eq!(
            code(judge_ledger(
                rail(),
                WalletKind::Native,
                &d,
                Err("Native account not found".into())
            ))
            .as_deref(),
            Some("wallet_unreadable")
        );
        for kind in [
            WalletKind::Native,
            WalletKind::Simple,
            WalletKind::Standard,
            WalletKind::Custom,
        ] {
            assert_eq!(
                code(judge_ledger(rail(), kind, &d, ledger(true, 1, 1))).as_deref(),
                Some("wallet_paused"),
                "{kind:?}"
            );
            assert!(judge_ledger(rail(), kind, &d, ledger(false, 1, 1)).is_ok());
        }
        // A Native wallet is an account of the owner: the agent key and the server key must
        // be its signers.
        assert_eq!(
            code(judge_ledger(
                rail(),
                WalletKind::Native,
                &d,
                ledger(false, 0, 1)
            ))
            .as_deref(),
            Some("agent_not_signer")
        );
        assert_eq!(
            code(judge_ledger(
                rail(),
                WalletKind::Native,
                &d,
                ledger(false, 1, 0)
            ))
            .as_deref(),
            Some("server_not_signer")
        );
        // A contract wallet's agent has no account and no signer seat: the contract checks
        // its signature itself.
        for kind in [WalletKind::Simple, WalletKind::Standard, WalletKind::Custom] {
            assert!(judge_ledger(rail(), kind, &d, ledger(false, 0, 0)).is_ok());
        }
    }
}
