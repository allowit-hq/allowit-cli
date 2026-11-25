//! `allowit policy build POLICY_ID`: build the wallet code of a Custom policy from the exact
//! source that the owner signed and hand the code to the server, which uploads it when the owner
//! deploys.
//!
//! The command reads the source that the server admitted with the build ticket and refuses a
//! source other than the one that the owner signed: its hash comes from the build command that
//! the app shows, or from the owner session. The rail of the policy builds the code. The owner's
//! one creation signature then names the code by its hash.
use crate::{
    commands::policy_help::is_policy_id,
    ctx::Ctx,
    error::{Error, Result},
    owner,
    routing::{self, RailInfo},
    settings, term,
};
use allowit_sdk::custom_build;
use serde_json::json;
use std::path::{Path, PathBuf};

pub(crate) const EXAMPLES: &str = "\
Examples:
  allowit policy build POLICY_ID --ticket TICKET --source-hash SHA256
  allowit policy build POLICY_ID --owner-key owner --origin http://localhost:5173 --out wallet.code";

/// Build the wallet code of a Custom policy from the source that the owner signed
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// The id of the Custom policy
    pub policy_id: String,
    /// Build ticket of the policy, from the build command that the app shows
    #[arg(long, value_name = "TICKET", requires = "source_hash")]
    pub ticket: Option<String>,
    /// SHA-256 of the source that the owner signed, from the same build command
    #[arg(long, value_name = "SHA256", requires = "ticket")]
    pub source_hash: Option<String>,
    /// Named local owner key: read the ticket and the signed source hash with the owner session
    #[arg(long, requires = "origin", conflicts_with = "ticket")]
    pub owner_key: Option<String>,
    /// Owner sign-in Origin admitted by the backend
    #[arg(long, env = "ALLOWIT_OWNER_ORIGIN", hide_env_values = true)]
    pub origin: Option<String>,
    /// Server public key for the owner sign-in; the default is the key that rail trust pinned
    #[arg(long, value_name = "PUBLIC_KEY")]
    pub server_key: Option<String>,
    /// Write the built wallet code to this file
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

/// The server key that this run trusts for `server` on `rail`: the key of the command, or the
/// key that `rail trust` pinned. Two keys that differ stop the build.
fn trusted_key(cx: &mut Ctx, rail: &RailInfo, server: &str, named: Option<&str>) -> Result<String> {
    let origin = crate::config::parse_origin(server)?;
    let pinned = cx
        .settings()?
        .rail(rail.id)
        .server_authorities
        .get(&origin)
        .cloned();
    let key = match (named, pinned) {
        (Some(named), Some(pinned)) if named != pinned => {
            return Err(Error::config(format!(
                "--server-key differs from the server key that allowit rail trust pinned for {origin} on {}; compare both with the key that the app shows",
                rail.id
            )));
        }
        (Some(named), _) => named.to_owned(),
        (None, Some(pinned)) => pinned,
        (None, None) => {
            return Err(Error::config(format!(
                "no trusted server key for {origin} on {}; pass --server-key PUBLIC_KEY from the app, or run allowit rail trust {origin} PUBLIC_KEY",
                rail.id
            )));
        }
    };
    // The key must be a public identity of the policy's rail.
    allowit_sdk::config::ServerAuthority::new(rail.id, &origin, &key)?;
    Ok(key)
}

/// One build at a time in the shared build folder. The lock ends with the process.
struct BuildLock(#[allow(dead_code)] std::fs::File);
fn lock(work: &Path) -> Result<BuildLock> {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::create(work.join("lock"))
        .map_err(|_| Error::config(format!("cannot lock {}", work.display())))?;
    // SAFETY: `flock` takes a descriptor that `file` keeps open for the life of the lock.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(Error::config(format!("cannot lock {}", work.display())));
    }
    Ok(BuildLock(file))
}

/// A SHA-256 in lowercase hexadecimal text.
fn is_hash(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) fn run(a: Args, cx: &mut Ctx) -> Result<i32> {
    if !is_policy_id(&a.policy_id) {
        return Err(Error::usage("POLICY_ID is not a policy id"));
    }
    if a.source_hash.as_deref().is_some_and(|hash| !is_hash(hash)) {
        return Err(Error::usage(
            "--source-hash is 64 lowercase hexadecimal digits",
        ));
    }
    let server = cx.server_url()?;
    let (ticket, source_hash) = match (&a.ticket, &a.source_hash, &a.owner_key, &a.origin) {
        (Some(ticket), Some(hash), _, _) => (ticket.clone(), hash.clone()),
        (None, None, Some(name), Some(origin)) => {
            cx.out
                .progress("Reading the build ticket with the owner session");
            // The owner signs in to the server whose key this run trusts.
            let rail = cx.rail()?;
            let key = trusted_key(cx, rail, &server, a.server_key.as_deref())?;
            let origin_of_server = crate::config::parse_origin(&server)?;
            let authority =
                allowit_sdk::config::ServerAuthority::new(rail.id, &origin_of_server, &key)?;
            let session = owner::load_for_authority(cx, name, authority)?;
            let view = custom_build::owner_view(&session.context(origin), &a.policy_id)?;
            if !is_hash(&view.source_hash) {
                return Err(Error::config("The server returned an invalid source hash"));
            }
            (view.ticket, view.source_hash)
        }
        _ => {
            return Err(Error::usage(
                "pass --ticket TICKET --source-hash SHA256 from the app's build command, or --owner-key NAME --origin ORIGIN to read them with the owner session",
            ));
        }
    };
    cx.out.progress("Reading the admitted policy source");
    let admission = custom_build::fetch(&server, &a.policy_id, &ticket)?;
    let rail = routing::info(&admission.network)?;
    // Nothing compiles unless the server's source is the source that the owner signed.
    admission.check_source(&source_hash)?;
    let work = cx.dir()?.join("custom-build");
    settings::private_dir(&work)?;
    let _lock = lock(&work)?;
    let built = custom_build::build(rail.id, &admission.source, &work)?;
    let code_hash = custom_build::hex(&built.code_hash);
    if let Some(out) = &a.out {
        std::fs::write(out, &built.code)
            .map_err(|_| Error::usage(format!("cannot write {}", out.display())))?;
    }
    cx.out.progress("Handing the wallet code to the server");
    custom_build::submit(&server, &a.policy_id, &ticket, &built)?;
    let next = "deploy the wallet: the server uploads this code, and the owner signs its creation";
    if cx.out.json {
        cx.out.json_value(&json!({
            "policyId": admission.policy_id,
            "network": rail.id,
            "sourceHash": source_hash,
            "codeHash": code_hash,
            "toolchain": built.toolchain,
            "out": a.out,
            "next": next,
        }));
        return Ok(0);
    }
    let mut rows = vec![
        ("policy".to_owned(), admission.policy_id.clone()),
        ("network".into(), rail.id.into()),
        ("source hash".into(), source_hash),
        ("code hash".into(), code_hash),
        ("toolchain".into(), built.toolchain.clone()),
    ];
    if let Some(out) = &a.out {
        rows.push(("wallet code".into(), out.display().to_string()));
    }
    rows.push(("next".into(), next.into()));
    let width = rows.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    let padded: Vec<_> = rows
        .into_iter()
        .map(|(key, text)| (format!("{key:width$}"), text))
        .collect();
    cx.out.stdout.push_str(&term::render(&padded));
    Ok(0)
}
