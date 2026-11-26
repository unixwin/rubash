//! Regression pins for rubash#1274: `exec` dropped exported functions
//! (BASH_FUNC_* entries) from the child environment.
//!
//! Plain spawn serialized exported functions fine; the exec path built
//! the child env from a snapshot that omitted them, so the replacement
//! process lost every exported function while ordinary exported vars
//! survived. GNU bash 5.3 (variables.c exported function serialization:
//! `BASH_FUNC_<name>%%` entries are part of the environment on every
//! process creation, exec included). Issue repro (3/3 stable):
//!   f() { echo F; }; export -f f; exec bash -c 'f'
//!   GNU: F (rc 0).  Old niu: `f: command not found` (rc 127).
//!   f() { echo F; }; export -f f; exec /usr/bin/env | grep BASH_FUNC
//!   GNU: 1 entry.  Old niu: 0 entries (entry absent, not malformed).
//!
//! The pins use this binary itself as the exec target (its function
//! import is the same code path GNU exercises) plus a /usr/bin/env leg
//! that inspects the raw environment exactly like the issue did.

use std::process::Command;

fn rubash(script: &str, envs: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c").arg(script);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn issue1274_exec_child_env_carries_bash_func_entries() {
    if cfg!(windows) {
        return; // /usr/bin/env leg is unix-shaped; the self-exec legs below pin the same contract portably.
    }
    let (stdout, _stderr, code) = rubash("f() { echo F; }; export -f f; exec /usr/bin/env", &[]);
    assert_eq!(code, Some(0), "exec /usr/bin/env must succeed");
    assert!(
        stdout.contains("BASH_FUNC_f%%="),
        "child env must carry the BASH_FUNC_f%% entry after exec, got: {stdout:?}"
    );
}

#[test]
fn issue1274_exec_into_shell_runs_exported_function() {
    let self_path = env!("CARGO_BIN_EXE_rubash");
    let (stdout, _stderr, code) = rubash(
        "f() { echo FUNC_F; }; export -f f; exec \"$RUBASH_SELF\" -c 'f'",
        &[("RUBASH_SELF", self_path)],
    );
    assert_eq!(code, Some(0), "exec'd child must run the exported function");
    assert_eq!(stdout.trim(), "FUNC_F");
}

#[test]
fn issue1274_exec_multi_function_and_var_controls() {
    let self_path = env!("CARGO_BIN_EXE_rubash");
    // Multi-function body: b calls a; both exported (issue leg 2).
    let (stdout, _stderr, code) = rubash(
        "a() { echo A; }; b() { echo B; a; }; export -f a; export -f b; \
         exec \"$RUBASH_SELF\" -c 'b'",
        &[("RUBASH_SELF", self_path)],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "B\nA\n", "both exported functions survive exec");

    // Control from the issue: ordinary exported vars always survived —
    // pin so the exec env rebuild cannot regress them either.
    let (stdout2, _e2, code2) = rubash(
        "v=hello; export v; exec \"$RUBASH_SELF\" -c 'echo V=$v'",
        &[("RUBASH_SELF", self_path)],
    );
    assert_eq!(code2, Some(0));
    assert_eq!(stdout2.trim(), "V=hello");
}
