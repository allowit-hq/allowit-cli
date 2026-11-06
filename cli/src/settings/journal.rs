//! The local request journal. `exec` records each request before it leaves
//! the process, so `status ID` finds its rail and wallet and `history` lists
//! it. Records are private files in `requests/XX/`, one per request ID,
//! where XX is the first byte of the ID's SHA-256 in hex. Each of the 256
//! folders is one private store of at most 1,000 records, so the journal
//! holds up to 256,000 requests.
use crate::error::{Error, Result};
use allowit_sdk_engine::Store;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Entry {
    pub request_id: String,
    pub command: String,
    pub rail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet: Option<String>,
    /// The policy that the rail resolved for the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_origin: Option<String>,
    pub op: String,
    pub to: String,
    pub amount: String,
    pub created_at: u64,
    /// The last exit code, or `None` before the first result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
}

/// The folder of one request ID.
fn shard(id: &str) -> String {
    allowit_sdk_engine::digest(id.as_bytes())[..2].to_owned()
}

fn open_shard(dir: &Path, shard: &str) -> Result<Store> {
    super::private_dir(&dir.join("requests"))?;
    Store::open(&dir.join("requests").join(shard))
        .map_err(|_| Error::config("the request journal is unsafe or locked"))
}

fn open(dir: &Path, id: &str) -> Result<Store> {
    open_shard(dir, &shard(id))
}

/// Holds the lock of a request's folder, as another process does.
#[cfg(test)]
pub(crate) fn lock(dir: &Path, id: &str) -> Result<Store> {
    open(dir, id)
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn load(dir: &Path, id: &str) -> Result<Option<Entry>> {
    if !dir.join("requests").join(shard(id)).exists() {
        return Ok(None);
    }
    open(dir, id)?.load(id)
}

/// Records a new request, or checks that an existing record has the same
/// intent. A request ID never names two different requests.
pub(crate) fn record(dir: &Path, entry: &Entry) -> Result<()> {
    let store = open(dir, &entry.request_id)?;
    if let Some(old) = store.load::<Entry>(&entry.request_id)? {
        if old.command != entry.command
            || old.rail != entry.rail
            || old.wallet != entry.wallet
            || old.policy != entry.policy
            || (old.profile_version.is_some()
                && entry.profile_version.is_some()
                && old.profile_version != entry.profile_version)
            || (old.server_origin.is_some()
                && entry.server_origin.is_some()
                && old.server_origin != entry.server_origin)
            || old.op != entry.op
            || old.to != entry.to
            || old.amount != entry.amount
        {
            return Err(Error::usage(format!(
                "request ID {} already names a different request; use a new --request-id",
                entry.request_id
            )));
        }
        return Ok(());
    }
    store.save(&entry.request_id, entry)
}

pub(crate) fn finish(dir: &Path, id: &str, exit: i32) -> Result<()> {
    let store = open(dir, id)?;
    if let Some(mut entry) = store.load::<Entry>(id)? {
        entry.exit = Some(exit);
        store.save(id, &entry)?;
    }
    Ok(())
}

/// Every record, newest first.
pub(crate) fn entries(dir: &Path) -> Result<Vec<Entry>> {
    let requests = dir.join("requests");
    if !requests.exists() {
        return Ok(vec![]);
    }
    let unreadable = || Error::config("the request journal is unsafe or unreadable");
    let mut shards = vec![];
    for item in std::fs::read_dir(&requests).map_err(|_| unreadable())? {
        let name = item.map_err(|_| unreadable())?.file_name();
        let name = name.to_str().ok_or_else(unreadable)?;
        let valid = name.len() == 2
            && name
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !valid {
            return Err(unreadable());
        }
        shards.push(name.to_owned());
    }
    let mut all: Vec<Entry> = vec![];
    for shard in shards {
        all.extend(open_shard(dir, &shard)?.entries::<Entry>()?);
    }
    all.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.request_id.cmp(&b.request_id))
    });
    Ok(all)
}

/// A fresh request ID: `req-` and 16 hex digits of OS randomness.
pub(crate) fn new_id() -> Result<String> {
    let mut bytes = [0u8; 8];
    getrandom(&mut bytes)?;
    Ok(format!(
        "req-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

fn getrandom(bytes: &mut [u8]) -> Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(bytes))
        .map_err(|_| Error::config("operating-system randomness is unavailable"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn entry(id: &str, at: u64) -> Entry {
        Entry {
            request_id: id.into(),
            command: "exec".into(),
            rail: "stellar:testnet".into(),
            wallet: Some("CWALLET".into()),
            policy: Some("policy-1@digest".into()),
            op: "transfer".into(),
            to: "GDEST".into(),
            amount: "XLM:5".into(),
            profile_version: None,
            server_origin: None,
            created_at: at,
            exit: None,
        }
    }
    #[test]
    fn records_bind_one_intent_and_list_newest_first() {
        let dir = crate::settings::temp_dir("journal");
        assert!(entries(&dir).unwrap().is_empty());
        assert!(load(&dir, "pay-0001").unwrap().is_none());
        record(&dir, &entry("pay-0001", 10)).unwrap();
        record(&dir, &entry("pay-0002", 20)).unwrap();
        record(&dir, &entry("pay-0001", 99)).unwrap();
        type Change = (&'static str, fn(&mut Entry));
        let changes: [Change; 7] = [
            ("command", |e| e.command = "status".into()),
            ("rail", |e| e.rail = "stellar:pubnet".into()),
            ("wallet", |e| e.wallet = Some("COTHER".into())),
            ("policy", |e| e.policy = Some("policy-2@digest".into())),
            ("op", |e| e.op = "swap".into()),
            ("to", |e| e.to = "GOTHER".into()),
            ("amount", |e| e.amount = "XLM:6".into()),
        ];
        for (field, change) in changes {
            let mut other = entry("pay-0001", 10);
            change(&mut other);
            let err = record(&dir, &other).unwrap_err();
            assert_eq!(err.code, 2, "a changed {field} must not reuse the ID");
        }
        finish(&dir, "pay-0001", 0).unwrap();
        let all = entries(&dir).unwrap();
        assert_eq!(all[0].request_id, "pay-0002");
        assert_eq!(all[1].exit, Some(0));
        let id = new_id().unwrap();
        assert!(crate::request::request_id_valid(&id), "{id}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// More requests than one private store holds stay readable, and each
    /// keeps its binding.
    #[test]
    fn the_journal_outgrows_one_store() {
        let dir = crate::settings::temp_dir("journal-large");
        for i in 0..1_001u64 {
            record(&dir, &entry(&format!("pay-{i:05}"), i)).unwrap();
        }
        let all = entries(&dir).unwrap();
        assert_eq!(all.len(), 1_001);
        assert_eq!(all[0].request_id, "pay-01000");
        let mut other = entry("pay-00007", 7);
        other.amount = "XLM:6".into();
        assert_eq!(record(&dir, &other).unwrap_err().code, 2);
        assert_eq!(load(&dir, "pay-00007").unwrap().unwrap().created_at, 7);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
