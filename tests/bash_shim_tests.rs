//! `bash` face shim behaviour (src/bin/bash.rs).
//!
//! The shim is built only with `--features windows-shim` (see Cargo.toml):
//! on Unix the same face is a hardlink/symlink to the host binary, so there
//! is nothing to forward. Gating here keeps `cargo test` green on Unix
//! without the binary while Windows packaging still exercises it.
#![cfg(feature = "windows-shim")]

use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn bash_shim_forwards_command_and_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_bash"))
        .env("NIU_SHELL", env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "printf shim-ok"])
        .output()
        .expect("run bash shim");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "shim-ok");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn bash_shim_prefers_sibling_niu_over_ambient_shell() {
    let root = Path::new("target").join("bash-shim-sibling-test");
    let bin = root.join("winuxcmd").join("usr").join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_rubash"), root.join("niu.exe")).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_bash"), bin.join("bash.exe")).unwrap();

    let output = Command::new(bin.join("bash.exe"))
        .env("SHELL", "C:/Windows/System32/cmd.exe")
        .args(["-c", "printf sibling-ok"])
        .output()
        .expect("run sibling bash shim");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "sibling-ok");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn bash_shim_forwards_exit_code() {
    let status = Command::new(env!("CARGO_BIN_EXE_bash"))
        .env("NIU_SHELL", env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "exit 7"])
        .status()
        .expect("run bash shim");

    assert_eq!(status.code(), Some(7));
}

#[test]
fn bash_shim_ignores_legacy_winuxsh_names() {
    // 1.5.0 winux purge (S1): the pre-rename fallback names are out of
    // the locate chain. With only the legacy names pointing at a real
    // shell — and a sibling `winuxsh.exe` next to the install tree —
    // the shim must fail to locate a shell (exit 127) instead of
    // launching through any of them.
    let root = Path::new("target").join("bash-shim-legacy-test");
    let bin = root.join("winuxcmd").join("usr").join("bin");
    let empty_path = root.join("empty-path");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&empty_path).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_rubash"), root.join("winuxsh.exe")).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_bash"), bin.join("bash.exe")).unwrap();

    let output = Command::new(bin.join("bash.exe"))
        .env("WINUXSH_SHELL", env!("CARGO_BIN_EXE_rubash"))
        .env("WINUXSH", env!("CARGO_BIN_EXE_rubash"))
        .env_remove("NIU_SHELL")
        .env_remove("SHELL")
        .env("PATH", &empty_path)
        .args(["-c", "printf legacy-ok"])
        .output()
        .expect("run bash shim");

    assert_eq!(output.status.code(), Some(127));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot locate niu.exe"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
