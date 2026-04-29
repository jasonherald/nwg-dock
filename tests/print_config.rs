//! Integration test: `--print-config` produces TOML matching the merged
//! config (CLI + file + defaults). Hermetic — invokes the dock binary as
//! a subprocess; no compositor or display required, since
//! `--print-config` exits before any GTK / Wayland side effects.

use std::io::Write;
use std::process::Command;

fn dock_bin() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target");
    p.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    p.push("nwg-dock");
    p
}

#[test]
fn print_config_uses_file_value_when_cli_absent() {
    let mut cfg = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    cfg.write_all(
        br#"
[appearance]
icon-size = 96

[layout]
position = "left"
"#,
    )
    .unwrap();

    let output = Command::new(dock_bin())
        .args(["--config", cfg.path().to_str().unwrap(), "--print-config"])
        .output()
        .expect("dock binary should run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let out = String::from_utf8(output.stdout).unwrap();
    assert!(out.contains("icon-size = 96"), "got:\n{}", out);
    assert!(out.contains(r#"position = "left""#), "got:\n{}", out);
}

#[test]
fn print_config_cli_explicit_overrides_file() {
    let mut cfg = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    cfg.write_all(
        br#"
[appearance]
icon-size = 96
"#,
    )
    .unwrap();

    let output = Command::new(dock_bin())
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--icon-size",
            "32",
            "--print-config",
        ])
        .output()
        .expect("dock binary should run");
    assert!(output.status.success());

    let out = String::from_utf8(output.stdout).unwrap();
    assert!(out.contains("icon-size = 32"), "got:\n{}", out);
}

#[test]
fn print_config_with_no_file_uses_defaults() {
    let output = Command::new(dock_bin())
        .args(["--config", "/nonexistent/zzz.toml", "--print-config"])
        .output()
        .expect("dock binary should run");
    assert!(output.status.success());

    let out = String::from_utf8(output.stdout).unwrap();
    assert!(out.contains("icon-size = 48"), "got:\n{}", out); // built-in default
}

#[test]
fn print_config_with_malformed_file_exits_nonzero() {
    let mut cfg = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    cfg.write_all(b"[behavior\nautohide = true").unwrap();

    let output = Command::new(dock_bin())
        .args(["--config", cfg.path().to_str().unwrap(), "--print-config"])
        .output()
        .expect("dock binary should run");
    assert!(!output.status.success(), "should exit nonzero on bad TOML");
}
