//! Records the git revision of this build for `allowit --version`, so a
//! recording can prove which source built the binary. The revision is the
//! short commit hash, with `-dirty` when the working tree differs from that
//! commit. `ALLOWIT_BUILD_REVISION` sets it for a build without a git
//! checkout; without either, the revision is `unknown`.
use std::{path::Path, process::Command};

const OVERRIDE: &str = "ALLOWIT_BUILD_REVISION";

fn main() {
    let package = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let revision = std::env::var(OVERRIDE)
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| revision_text(text))
        .or_else(|| git_revision(&package))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=ALLOWIT_BUILD_REVISION={revision}");
    println!("cargo:rerun-if-env-changed={OVERRIDE}");
    // The revision changes with the commit and with every change of the
    // sources, so the script runs again after either.
    println!("cargo:rerun-if-changed={package}");
    for file in ["../Cargo.toml", "../Cargo.lock"] {
        println!("cargo:rerun-if-changed={package}/{file}");
    }
    for (directory, files) in [
        ("--absolute-git-dir", &["HEAD", "index"][..]),
        ("--git-common-dir", &["packed-refs"][..]),
    ] {
        if let Some(directory) = git(&package, &["rev-parse", directory]) {
            let directory = Path::new(&package).join(directory);
            for file in files {
                println!("cargo:rerun-if-changed={}", directory.join(file).display());
            }
            if files.contains(&"packed-refs")
                && let Some(branch) = git(&package, &["symbolic-ref", "-q", "HEAD"])
            {
                println!(
                    "cargo:rerun-if-changed={}",
                    directory.join(branch).display()
                );
            }
        }
    }
}

/// A revision is 1 to 64 characters from `0-9 A-Z a-z . _ -`, so it prints as one word.
fn revision_text(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The trimmed stdout of a successful git command in `directory`.
fn git(directory: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

/// The short hash of HEAD, with `-dirty` when a tracked file differs from
/// HEAD or a file that git does not ignore is untracked.
fn git_revision(directory: &str) -> Option<String> {
    let hash = git(directory, &["rev-parse", "--short=12", "HEAD"]).filter(|v| revision_text(v))?;
    // A tree whose state cannot be read is not reported as clean.
    let dirty =
        git(directory, &["status", "--porcelain"]).is_none_or(|changes| !changes.is_empty());
    Some(if dirty { format!("{hash}-dirty") } else { hash })
}
