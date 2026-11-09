//! The vendor-boundary eval: the CLI names no rail or vendor and reaches every
//! rail through the SDK plugin registry. Requires Node 22 or later.
use std::{path::Path, process::Command};

fn scan(arg: Option<&str>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut command = Command::new("node");
    command.current_dir(&root).arg("scripts/vendor-boundary.mjs");
    command.args(arg);
    let output = command
        .output()
        .expect("the vendor-boundary eval requires node on PATH");
    assert!(
        output.status.success(),
        "vendor-boundary {}failed:\n{}{}",
        arg.map(|a| format!("{a} ")).unwrap_or_default(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn scanner_self_test_passes() {
    scan(Some("--self-test"));
}

#[test]
fn the_cli_names_no_vendor() {
    scan(None);
}

#[test]
fn the_scan_fails_when_a_vendor_name_is_added() {
    scan(Some("--probe"));
}
