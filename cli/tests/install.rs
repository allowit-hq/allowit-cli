//! `install.sh`: a release binary is installed only when its SHA-256 matches the release's
//! list, and a dry run changes nothing. The release files come from a local folder, so the
//! test uses no network.
#![cfg(unix)]
use std::{path::Path, process::Command};

fn script() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../install.sh")
}

fn sh(command: &str, dir: &Path) -> String {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "{command}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// A release folder with one binary for this system and its checksum list.
fn release(root: &Path) -> std::path::PathBuf {
    let release = root.join("release");
    std::fs::create_dir_all(&release).unwrap();
    std::fs::write(
        release.join("allowit"),
        "#!/bin/sh\necho 'allowit 9.9.9 (0123456789ab)'\n",
    )
    .unwrap();
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        _ => "x86_64",
    };
    let triple = match std::env::consts::OS {
        "macos" => format!("{arch}-apple-darwin"),
        _ => format!("{arch}-unknown-linux-gnu"),
    };
    let asset = format!("allowit-{triple}.tar.gz");
    sh(
        &format!("chmod 755 allowit && tar -czf {asset} allowit && rm allowit"),
        &release,
    );
    let sum = sh(
        &format!("(sha256sum {asset} 2>/dev/null || shasum -a 256 {asset}) | cut -d' ' -f1"),
        &release,
    );
    std::fs::write(release.join("SHA256SUMS"), format!("{sum}  {asset}\n")).unwrap();
    release
}

fn path_with(dir: &Path) -> std::ffi::OsString {
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).unwrap()
}

fn install(root: &Path, release: &Path, extra: &[(&str, &str)]) -> std::process::Output {
    let mut command = Command::new("sh");
    command
        .arg(script())
        .env("HOME", root)
        .env("ALLOWIT_INSTALL_DIR", root.join("bin"))
        // The install directory is on PATH, so a root run of this test links nothing into
        // /usr/local/bin.
        .env("PATH", path_with(&root.join("bin")))
        .env(
            "ALLOWIT_RELEASE_URL",
            format!("file://{}", release.display()),
        )
        // A failed release path must not start a source build in a test.
        .env("ALLOWIT_NO_DEPS", "1")
        .env("ALLOWIT_REF", "0000000000000000000000000000000000000000");
    for (name, value) in extra {
        command.env(name, value);
    }
    command.output().unwrap()
}

#[test]
fn a_release_binary_is_installed_only_when_its_checksum_matches() {
    if Command::new("curl").arg("--version").output().is_err() {
        return;
    }
    let root = std::env::temp_dir().join(format!("allowit-install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let release = release(&root);
    let installed = root.join("bin/allowit");

    // A dry run names the plan and changes nothing.
    let dry = install(&root, &release, &[("ALLOWIT_DRY_RUN", "1")]);
    assert!(dry.status.success(), "{dry:?}");
    assert!(String::from_utf8_lossy(&dry.stderr).contains("would install the release binary"));
    assert!(!installed.exists());

    // The matching checksum installs the binary and prints its version with the revision.
    let done = install(&root, &release, &[]);
    assert!(done.status.success(), "{done:?}");
    assert_eq!(
        String::from_utf8_lossy(&done.stdout).trim(),
        "allowit 9.9.9 (0123456789ab)"
    );
    let log = String::from_utf8_lossy(&done.stderr).into_owned();
    assert!(log.contains("the SHA-256 matches the release"), "{log}");
    assert!(installed.exists());

    // A file that differs from the release's list is refused, and the installed binary stays.
    let before = std::fs::read(&installed).unwrap();
    let sums = std::fs::read_to_string(release.join("SHA256SUMS")).unwrap();
    let (sum, name) = sums.trim().split_once("  ").unwrap();
    let other = format!(
        "{}{}",
        if sum.starts_with('0') { "1" } else { "0" },
        &sum[1..]
    );
    std::fs::write(release.join("SHA256SUMS"), format!("{other}  {name}\n")).unwrap();
    let refused = install(&root, &release, &[]);
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    let log = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(log.contains("Nothing was installed"), "{log}");
    assert_eq!(std::fs::read(&installed).unwrap(), before);

    // A release without a binary for this system falls back to a source build, which this
    // test stops before it starts: the commit does not exist.
    std::fs::write(release.join("SHA256SUMS"), "00  allowit-other.tar.gz\n").unwrap();
    let fallback = install(&root, &release, &[]);
    assert_eq!(fallback.status.code(), Some(1));
    let log = String::from_utf8_lossy(&fallback.stderr).into_owned();
    assert!(log.contains("building from source"), "{log}");
    // A requested release without a binary builds that release from source, not main.
    let pinned = Command::new("sh")
        .arg(script())
        .env("HOME", &root)
        .env("ALLOWIT_INSTALL_DIR", root.join("bin"))
        .env("PATH", path_with(&root.join("bin")))
        .env(
            "ALLOWIT_RELEASE_URL",
            format!("file://{}", release.display()),
        )
        .env("ALLOWIT_VERSION", "v9.9.9")
        .env("ALLOWIT_NO_DEPS", "1")
        .env("ALLOWIT_DRY_RUN", "1")
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&pinned.stderr).into_owned();
    assert!(log.contains("(v9.9.9)"), "{log}");
    assert!(!log.contains("(main)"), "{log}");
    let _ = std::fs::remove_dir_all(&root);
}
