//! rubash#1146 — No diagnostic when BASH_ENV names a directory.
//!
//! GNU shell.c / flags: a non-interactive shell reads $BASH_ENV at
//! startup; a startup file that cannot be read because it is a
//! directory produces a diagnostic (`<path>: is a directory`) and
//! execution continues with rc=0. Silence makes a mistyped BASH_ENV
//! indistinguishable from a successfully loaded empty file.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` string with
//! BASH_ENV pointing at a directory).

use std::process::Command;

#[test]
fn bash_env_naming_a_directory_is_diagnosed() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("BASH_ENV", "/tmp")
        .arg("-c")
        .arg("echo ok")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(stdout, "ok\n");
    assert!(
        stderr.contains("/tmp") && stderr.to_lowercase().contains("is a directory"),
        "stderr: {stderr}"
    );
    assert_eq!(output.status.code(), Some(0));
}
