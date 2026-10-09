//! niubash#452 regression: an assignment word followed by `(` parses as the
//! GNU assignment-prefix + subshell shape instead of a syntax error.
//!
//! The eco-audit corpus (scripts/harvest/snapshots/wt92-local-20261005/) recorded
//! GNU `bash -n` ACCEPTING these lines while rubash rejected them with
//! `syntax error near unexpected token '('`:
//!
//! - oh-my-bash cli.bash:      `x=plugin::@(disable|enable|load)` — the word
//!   breaks at the unquoted `(` (extglob gate closed), leaving the assignment
//!   `x=plugin::@` followed by a subshell `(disable|enable|load)`.
//! - same shape with a plain value: `pat=-*@([aAbcelmnNosu]|t-)`.
//!
//! GNU semantics (bash manual, "assignment statements may precede a compound
//! command"): the prefix applies to the environment of the compound command
//! only, and is gone when the subshell ends.

use std::process::Command;

fn shell_bin() -> std::path::PathBuf {
    // NOTE: the `bash` bin is a thin forwarder to the INSTALLED Niubash
    // (src/bin/bash.rs) — testing through it would exercise the old engine.
    // The real engine binary is `rubash`.
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return std::path::PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

fn run(script: &str) -> (i32, String, String) {
    let output = Command::new(shell_bin())
        .args(["-c", script])
        .output()
        .expect("run shell");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn parse_rc(script: &str) -> i32 {
    let output = Command::new(shell_bin())
        .args(["-n", "-c", script])
        .output()
        .expect("run shell -n");
    output.status.code().unwrap_or(-1)
}

#[test]
fn assignment_word_followed_by_subshell_parses() {
    // corpus lines: GNU accepts, rubash used to reject with `(' unexpected
    assert_eq!(parse_rc("x=a@(b|c)"), 0);
    assert_eq!(parse_rc("pat=-*@([aAbcelmnNosu]|t-)"), 0);
    assert_eq!(parse_rc("x=plugin::@(disable|enable|load)"), 0);
    assert_eq!(parse_rc("x=1 (echo hi)"), 0);
    assert_eq!(parse_rc("x=1 ( (echo hi) )"), 0);
}

#[test]
fn empty_value_and_genuine_errors_still_reject() {
    // `x= (` stays the compound-assignment ambiguity GNU rejects (#221)
    assert_eq!(parse_rc("x= (echo hi)"), 2);
    // a bare word before `(` is GNU's function-def production, which demands
    // `name() {' — `foo (echo hi)` blames `echo` in GNU too
    assert_eq!(parse_rc("foo (echo hi)"), 2);
    // redirection between prefix and compound has no GNU-accept evidence
    assert_eq!(parse_rc("x=1 < in.txt (echo hi)"), 2);
}

#[test]
fn prefix_applies_to_subshell_environment_only() {
    let (code, stdout, stderr) = run("x=1 (echo ${x:-unset})");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "1");

    // the prefix is gone after the subshell ends
    let (code, stdout, stderr) = run("x=outer (x=inner; echo $x); echo [$x]");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "inner\n[]\n");

    let (_, stdout, _) = run("x=1 (exit 3); echo $?"); // status still propagates
    assert_eq!(stdout.trim(), "3");
}
