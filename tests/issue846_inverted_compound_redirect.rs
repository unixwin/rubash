//! Issue rubash#846 ? `!` must not invert a compound command's own
//! redirection failure.
//!
//! GNU execute_cmd.c:854-883: a shell control structure's redirections run
//! in do_redirections BEFORE the body, and failure there is
//! EXECUTION_FAILURE ? `invert` only gates errexit/the ERR trap (:868), it
//! does NOT flip the return status. A simple command's redirect failure is
//! part of the command's result and `!` inverts it.
//!
//! Byte-verified against WSL GNU Bash 5.3.15(1) (2026-10-19).

use std::path::Path;
use std::process::Command;

fn rubash_in(script: &str, cwd: &Path) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash(script: &str) -> (String, String, Option<i32>) {
    rubash_in(script, &std::env::temp_dir())
}

/// The pinned matrix from the issue body (WSL GNU vs rubash).
#[test]
fn inverted_compound_redirect_failure_matrix() {
    let bad = "/nonexistent_dir_846/f";
    // Every compound form keeps rc=1 under `!`; only the simple command
    // inverts its own redirect failure to 0.
    let cases = [
        (format!("! {{ :; }} > {bad}"), 1),
        (format!("! {{ :; }} 2> {bad}"), 1),
        (format!("! ( :; ) > {bad}"), 1),
        (format!("! if true; then :; fi > {bad}"), 1),
        (format!("! while false; do :; done > {bad}"), 1),
        (format!("! for x in; do :; done > {bad}"), 1),
        (format!("! case x in *) :;; esac > {bad}"), 1),
        (format!("! (( 1 )) > {bad}"), 1),
        (format!("! [[ -d /nonexistent ]] > {bad}"), 1),
        (format!("! true > {bad}"), 0),
    ];
    for (script, want_rc) in cases {
        let (_, _, code) = rubash(&script);
        assert_eq!(code, Some(want_rc), "script: {script}");
    }
}

/// `!` still inverts a compound's *inner* failure (the body ran; its
/// status is a real command result).
#[test]
fn inverted_compound_inner_failure_still_flips() {
    let (_, _, code) = rubash("! { false; }");
    assert_eq!(code, Some(0));
    let (_, _, code) = rubash("! ( false )");
    assert_eq!(code, Some(0));
}

/// `set -e` interaction: the compound redirect failure is still a failure
/// for errexit (GNU's invert gates the trap/errexit door, not the status).
#[test]
fn inverted_compound_redirect_failure_under_errexit() {
    let script = "set -e\n! { :; } > /nonexistent_dir_846/f\necho unreachable";
    let (stdout, _, code) = rubash(script);
    // GNU: `!` gates the errexit door — the command's rc stays 1 but the
    // failure does not trigger errexit, so the script continues.
    assert_eq!(stdout, "unreachable\n");
    assert_eq!(code, Some(0));
}
