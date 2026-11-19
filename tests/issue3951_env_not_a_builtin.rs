//! Regression pins for rubash#3951: `env` is not a shell builtin.
//!
//! GNU bash has no `env` builtin (there is no builtins/env.def in
//! bash 5.3; `env` is a coreutils program, /usr/bin/env). rubash
//! registered `env` in its builtin table (62 builtins vs GNU's 61),
//! leaking it into every builtin-introspection face — `compgen -A
//! builtin` / `-A enabled`, `enable -n`, `builtin`, `hash -t` —
//! while the execution faces still treated it as external.
//!
//! Residual of closed #2538 / #2581: #2581's fix added the `builtin
//! env` dispatch arm ("add the arm or remove env from builtin
//! faces") instead of removing the faces, and #2538 fixed only the
//! `-A command` face. #3951 removes `env` from the builtin
//! registration entirely: it becomes an emulated external command
//! (registry class EmulatedExternal, still dispatched in-process by
//! `execute_env_command`, so plain `env` keeps working on platforms
//! without /usr/bin/env). The builtin faces now agree with GNU:
//! 61 builtins, `enable -n env` / `builtin env` fail rc 1 with
//! "not a shell builtin", `hash -t env` records /usr/bin/env.
//!
//! GNU oracle (bash 5.3.20, verified 2026-11-19): `compgen -A
//! builtin` lists 61 names; `compgen -A builtin env` / `-A enabled
//! env` print nothing, rc 1; `enable -n env` fails rc 1;
//! `builtin env` fails rc 1; `hash env; hash -t env` prints
//! /usr/bin/env, rc 0; `type env` prints `env is /usr/bin/env`.
//! Port of the issue's regression seed R140-B (12 checks) plus the
//! 61-count pins and the plain-`env` execution controls.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join("rubash")
}

struct Output {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

/// Run one `-c` script in a fresh shell with a controlled
/// environment (PATH=/usr/bin:/bin, empty HOME scratch dir), exactly
/// like the issue seed: no ambient hash entries, no BASH_ENV.
fn run(dir: &Path, script: &str) -> Output {
    let output = Command::new(shell_bin())
        .args(["-c", script])
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Output {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("rubash-3951-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn issue3951_compgen_builtin_and_enabled_exclude_env() {
    let dir = scratch("compgen");
    let out = run(&dir, "compgen -A builtin env; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n", "stdout: {}", out.stdout);
    assert!(out.stderr.is_empty(), "stderr: {}", out.stderr);
    let out = run(&dir, "compgen -A enabled env; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n", "stdout: {}", out.stdout);
    assert!(out.stderr.is_empty(), "stderr: {}", out.stderr);
    // No `env` line anywhere in the full lists.
    let out = run(&dir, "compgen -A builtin '' | grep -c '^env$'; echo rc=$?");
    assert_eq!(out.stdout, "0\nrc=1\n", "stdout: {}", out.stdout);
    let out = run(&dir, "compgen -A enabled '' | grep -c '^env$'; echo rc=$?");
    assert_eq!(out.stdout, "0\nrc=1\n", "stdout: {}", out.stdout);
}

#[test]
fn issue3951_builtin_count_is_gnu_61() {
    let dir = scratch("count");
    let out = run(&dir, "compgen -A builtin '' | wc -l");
    assert_eq!(out.stdout, "61\n", "stdout: {}", out.stdout);
    let out = run(&dir, "compgen -b | wc -l");
    assert_eq!(out.stdout, "61\n", "stdout: {}", out.stdout);
    let out = run(&dir, "enable -a | wc -l");
    assert_eq!(out.stdout, "61\n", "stdout: {}", out.stdout);
    let out = run(&dir, "enable -a");
    assert!(
        !out.stdout.contains("enable env\n"),
        "enable -a must not list env: {}",
        out.stdout
    );
}

#[test]
fn issue3951_enable_n_env_fails() {
    let dir = scratch("enable");
    let out = run(&dir, "enable -n env; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n", "stdout: {}", out.stdout);
    assert!(
        out.stderr.contains("not a shell builtin"),
        "stderr: {}",
        out.stderr
    );
    // The failed disable must not move env into the disabled list.
    let out = run(
        &dir,
        "enable -n env 2>/dev/null; compgen -A disabled env; echo rc=$?",
    );
    assert_eq!(out.stdout, "rc=1\n", "stdout: {}", out.stdout);
    let out = run(&dir, "compgen -A disabled env; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n", "stdout: {}", out.stdout);
}

#[test]
fn issue3951_builtin_env_fails() {
    let dir = scratch("builtin");
    let out = run(&dir, "builtin env >/dev/null; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n", "stdout: {}", out.stdout);
    assert!(
        out.stderr.contains("not a shell builtin"),
        "stderr: {}",
        out.stderr
    );
    let out = run(&dir, "builtin env FOO=bar printenv FOO; echo rc=$?");
    assert_eq!(out.stdout, "rc=1\n", "stdout: {}", out.stdout);
    assert!(
        out.stderr.contains("not a shell builtin"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn issue3951_hash_env_records_path_file() {
    let dir = scratch("hash");
    let out = run(&dir, "hash env; hash -t env; echo rc=$?");
    assert_eq!(out.stdout, "/usr/bin/env\nrc=0\n", "stdout: {}", out.stdout);
    assert!(out.stderr.is_empty(), "stderr: {}", out.stderr);
}

#[test]
fn issue3951_command_and_type_controls() {
    let dir = scratch("controls");
    // `env` is still a real external command.
    let out = run(&dir, "compgen -A command e | grep -q '^env$'; echo rc=$?");
    assert_eq!(out.stdout, "rc=0\n", "stdout: {}", out.stdout);
    let out = run(&dir, "type env; echo rc=$?");
    assert_eq!(
        out.stdout, "env is /usr/bin/env\nrc=0\n",
        "stdout: {}",
        out.stdout
    );
    let out = run(&dir, "command -v env; echo rc=$?");
    assert_eq!(out.stdout, "/usr/bin/env\nrc=0\n", "stdout: {}", out.stdout);
}

#[test]
fn issue3951_plain_env_execution_unchanged() {
    let dir = scratch("plain");
    // Plain `env` still prints the environment (in-process
    // emulation, identical output to /usr/bin/env here).
    let out = run(&dir, "FOO=bar env | grep '^FOO='");
    assert_eq!(out.stdout, "FOO=bar\n", "stdout: {}", out.stdout);
    // ... and still runs a command with assignments applied.
    let out = run(&dir, "env FOO=bar printenv FOO; echo rc=$?");
    assert_eq!(out.stdout, "bar\nrc=0\n", "stdout: {}", out.stdout);
    // `command env` still reaches the disk file.
    let out = run(&dir, "command env FOO=bar printenv FOO; echo rc=$?");
    assert_eq!(out.stdout, "bar\nrc=0\n", "stdout: {}", out.stdout);
}
