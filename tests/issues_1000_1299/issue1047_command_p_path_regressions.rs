//! rubash#1047: `command -p` uses the compiled-in default path for
//! LOOKUP ONLY; it must not leak that path into the executed
//! command's environment.
//!
//! GNU anchor: `builtins/command.def`: `-p` performs the search with
//! the default path value but leaves `$PATH` (and the child's
//! inherited environment) exactly as the caller set it. rubash
//! temporarily installed the default PATH and passed it on, so
//! `command -p sh -c 'echo $PATH'` printed the default path instead
//! of the caller's value.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.
#![cfg(unix)]

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The reported case: the child sees the caller's PATH verbatim.
#[test]
fn command_p_keeps_caller_path_in_child_env() {
    let (stdout, _, _) = rubash("PATH=/nonexistent_xyz; command -p sh -c 'echo PATH=$PATH'");
    assert_eq!(stdout, "PATH=/nonexistent_xyz\n");
}

/// Lookup itself still works under `-p` despite the bogus PATH.
#[test]
fn command_p_still_finds_programs_via_default_path() {
    let (stdout, _, _) =
        rubash("PATH=/nonexistent_xyz; command -p ls / >/dev/null; echo \"ls-rc=$?\"");
    assert_eq!(stdout, "ls-rc=0\n");
}

/// Nested form from the report: `command -p command -v ls` FAILS
/// under GNU, because the inner lookup runs with the caller's PATH.
#[test]
fn command_p_nested_lookup_uses_caller_path() {
    let (stdout, _, _) =
        rubash("PATH=/nonexistent_xyz; command -p command -v ls; echo \"nested-rc=$?\"");
    assert_eq!(stdout, "nested-rc=1\n");
}

/// Control that already agreed: without `-p`, the bogus PATH really
/// does make the command unfindable (rc 127).
#[test]
fn command_without_p_under_bogus_path_fails() {
    let (stdout, stderr, _) =
        rubash("PATH=/nonexistent_xyz; command sh -c 'echo hi'; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=127\n");
    assert!(stderr.contains("sh: command not found"), "stderr: {stderr}");
}
