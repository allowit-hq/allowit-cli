//! `allowit policy build POLICY_ID`: build the wallet code of a Custom policy from the exact
//! source that the server admitted, hand the code to the server and attest the build.
//!
//! The command reads the source with the server presignature and verifies the presignature
//! against the trusted server key before it compiles anything. The rail of the policy builds
//! the code. With a local owner key the command signs the build attestation; without one the
//! owner's wallet signs it in the app.
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
  allowit policy build POLICY_ID --ticket TICKET --server-key PUBLIC_KEY
  allowit policy build POLICY_ID --owner-key owner --origin http://localhost:5173 --out wallet.wasm";

/// Build the wallet code of a Custom policy from its admitted source and attest the build
#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    /// The id of the Custom policy
    pub policy_id: String,
    /// Build ticket of the policy, from the build command that the app shows
    #[arg(long, value_name = "TICKET")]
    pub ticket: Option<String>,
    /// Server public key from the build command; the default is the key that rail trust pinned
    #[arg(long, value_name = "PUBLIC_KEY")]
    pub server_key: Option<String>,
    /// Named local owner key: sign the build attestation here instead of in the app
    #[arg(long, requires = "origin")]
    pub owner_key: Option<String>,
    /// Owner sign-in Origin admitted by the backend
    #[arg(long, env = "ALLOWIT_OWNER_ORIGIN", hide_env_values = true)]
    pub origin: Option<String>,
    /// Write the built wallet code to this file, for policy deploy --executable
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

/// The server key that this run trusts for `server` on `rail`: the key of the build command,
/// or the key that `rail trust` pinned. Two keys that differ stop the build.
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
                "no trusted server key for {origin} on {}; pass --server-key PUBLIC_KEY from the app's build command, or run allowit rail trust {origin} PUBLIC_KEY",
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

pub(crate) fn run(a: Args, cx: &mut Ctx) -> Result<i32> {
    if !is_policy_id(&a.policy_id) {
        return Err(Error::usage("POLICY_ID is not a policy id"));
    }
    let server = cx.server_url()?;
    // A local owner key signs in for the ticket and signs the attestation.
    let local = match (&a.owner_key, &a.origin) {
        (Some(name), Some(origin)) => Some((owner::signer(cx, name)?, name.clone(), origin.clone())),
        _ => None,
    };
    let ticket = match (&a.ticket, &local) {
        (Some(ticket), _) => ticket.clone(),
        (None, Some((_, name, origin))) => {
            cx.out.progress("Reading the build ticket with the owner session");
            let session = owner::load_for_server(cx, name)?;
            custom_build::owner_view(&session.context(origin), &a.policy_id)?.ticket
        }
        (None, None) => {
            return Err(Error::usage(
                "pass --ticket TICKET from the app's build command, or --owner-key NAME to read it with the owner session",
            ));
        }
    };
    cx.out.progress("Reading the admitted policy source");
    let sealed = custom_build::fetch(&server, &a.policy_id, &ticket)?;
    let rail = routing::info(sealed.network())?;
    let key = trusted_key(cx, rail, &server, a.server_key.as_deref())?;
    // Nothing is compiled before the trusted server key signed this exact source.
    let admission = sealed.verify(&key)?;
    if let Some((signer, name, _)) = &local {
        if signer.network() != rail.id {
            return Err(Error::config(format!(
                "the owner key {name} belongs to {}, and the policy is on {}",
                signer.network(),
                rail.id
            )));
        }
        if signer.address() != admission.owner {
            return Err(Error::config(format!(
                "the key {name} is not the owner of this policy; the owner is {}",
                admission.owner
            )));
        }
    }
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
    let attested = match &local {
        Some((signer, _, _)) => {
            cx.confirm("Attest this build as the owner of the policy")?;
            let attestation =
                custom_build::attest(signer, &admission.presignature, &built.code_hash)?;
            custom_build::register(&server, &a.policy_id, &ticket, &attestation)?.attested
        }
        None => false,
    };
    let next = if attested {
        "deploy the wallet"
    } else {
        "sign the build attestation in the app"
    };
    if cx.out.json {
        cx.out.json_value(&json!({
            "policyId": admission.policy_id,
            "network": rail.id,
            "sourceHash": admission.presignature.source_hash,
            "codeHash": code_hash,
            "toolchain": built.toolchain,
            "attested": attested,
            "out": a.out,
            "next": next,
        }));
        return Ok(0);
    }
    let mut rows = vec![
        ("policy".to_owned(), admission.policy_id.clone()),
        ("network".into(), rail.id.into()),
        ("source hash".into(), admission.presignature.source_hash.clone()),
        ("code hash".into(), code_hash),
        ("toolchain".into(), built.toolchain.clone()),
    ];
    if let Some(out) = &a.out {
        rows.push(("wallet code".into(), out.display().to_string()));
    }
    rows.push(("attested".into(), if attested { "yes" } else { "no" }.into()));
    rows.push(("next".into(), next.into()));
    let width = rows.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    let padded: Vec<_> = rows
        .into_iter()
        .map(|(key, text)| (format!("{key:width$}"), text))
        .collect();
    cx.out.stdout.push_str(&term::render(&padded));
    Ok(0)
}
