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
        .env("WINUXSH_SHELL", env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "printf shim-ok"])
        .output()
        .expect("run bash shim");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "shim-ok");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn bash_shim_prefers_sibling_winuxsh_over_ambient_shell() {
    let root = Path::new("target").join("bash-shim-sibling-test");
    let bin = root.join("winuxcmd").join("usr").join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_rubash"), root.join("winuxsh.exe")).unwrap();
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
        .env("WINUXSH_SHELL", env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "exit 7"])
        .status()
        .expect("run bash shim");

    assert_eq!(status.code(), Some(7));
}
