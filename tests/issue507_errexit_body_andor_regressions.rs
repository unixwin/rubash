//! Issue rubash#507 regression: under `set -e`, an exempt inner failure
//! (`&&`/`||` short-circuit arm, `!` pipeline) inside a compound body must
//! not kill the shell when the compound's aggregate status reaches the
//! top-level walker.
//!
//! GNU spec (bash 5.3.0 WSL probes, 2026-10-12; issue repro stdin-driven):
//!
//! ```text
//! $ printf '%s' 'set -e; if true; then false && true; fi; echo R' | bash -s
//! R                                  # rc 0
//! $ printf '%s' 'set -e; if true; then false; fi; echo R' | bash -s
//!                                    # rc 1: the plain inner failure exits
//! $ printf '%s' 'set -e; (false && true); echo R' | bash -s
//!                                    # rc 1: subshell aggregate adjudicated
//! $ printf '%s' 'set -e; f(){ false && true; }; f; echo R' | bash -s
//!                                    # rc 1: the call's status belongs to it
//! ```
//!
//! POSIX XCU 2.14: -e is ignored for any command of an AND-OR list other
//! than the last and for `!` pipelines; the exemption must survive the
//! compound boundary. GNU's execute_if_command (execute_cmd.c:5200 family)
//! hands the body status up with no errexit check of its own — one
//! adjudication per failure, at the site that produced it.
//!
//! Rust semantic owner: executor/ast_exec.rs `command_status_produced_by_body`
//! gating the post-compound ERR-trap fire and the errexit promotion.

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

/// Issue-driver shape: the script arrives on stdin, like the corpus replay.
fn run_stdin(script: &str) -> (Option<i32>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(script.as_bytes())
        .expect("write script to stdin");
    let output = child.wait_with_output().expect("wait for rubash");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

fn run_file(script: &str) -> (Option<i32>, String) {
    let dir = std::env::temp_dir().join(format!("rubash-issue507-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue507 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// The issue repro (stdin): the exempt `&&` arm inside the if body must not
/// exit the shell.
#[test]
fn issue_repro_if_body_andor_list_reaches_trailing_command() {
    let (code, stdout) = run_stdin("set -e; if true; then false && true; fi; echo R\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "R\n");

    let (code, stdout) = run_file("set -e; if true; then false && true; fi; echo R\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "R\n");
}

/// Every in-process body shape passes its aggregate through (GNU all rc 0,
/// all print R; matrix follows huck#676's measured table).
#[test]
fn exempt_body_failures_pass_through_every_construct() {
    let cases: [&str; 7] = [
        "set -e; if false; then :; elif true; then false && true; fi; echo R\n",
        "set -e; if true; then true; else false && true; fi; echo R\n",
        "set -e; for i in 1; do false && true; done; echo R\n",
        "set -e; i=0; while [ $i = 0 ]; do i=1; false && true; done; echo R\n",
        "set -e; i=0; until [ $i = 1 ]; do i=1; false && true; done; echo R\n",
        "set -e; case x in x) false && true;; esac; echo R\n",
        "set -e; if true; then ! true; fi; echo R\n",
    ];
    for script in cases {
        let (code, stdout) = run_stdin(script);
        assert_eq!(code, Some(0), "exited early: {script:?}");
        assert_eq!(stdout, "R\n", "wrong output: {script:?}");
    }
}

/// The aggregate status still propagates — only the exit is waived (GNU
/// prints `status=1`, rc 0).
#[test]
fn aggregate_status_survives_without_the_exit() {
    let (code, stdout) = run_stdin("set -e; if true; then false && true; fi; echo status=$?\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "status=1\n");
}

/// ERR shares the exemption: the compound's aggregate must not fire ERR for
/// an inner exempt failure (GNU prints only R, rc 0).
#[test]
fn err_trap_not_fired_at_pass_through_aggregate() {
    let (code, stdout) =
        run_stdin("set -e; trap 'echo E' ERR; if true; then false && true; fi; echo R\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "R\n");

    let (code, stdout) =
        run_stdin("trap 'echo E' ERR; for i in 1; do false && true; done; echo R\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "R\n");
}

/// What must keep exiting (GNU all rc 1, nothing printed after): the safety
/// half that stops the fix becoming "never adjudicate a compound".
#[test]
fn fresh_failures_at_this_level_still_exit() {
    let cases: [&str; 9] = [
        "set -e; if true; then false; fi; echo R\n",
        "set -e; { false; }; echo R\n",
        "set -e; (false && true); echo R\n",
        "set -e; f() { false && true; }; f; echo R\n",
        "set -e; true && false; echo R\n",
        "set -e; for i in 1; do false; done; echo R\n",
        "set -e; case x in x) false;; esac; echo R\n",
        "set -e; [[ 1 = 2 ]]; echo R\n",
        "set -e; (( 0 )); echo R\n",
    ];
    for script in cases {
        let (code, stdout) = run_stdin(script);
        assert_eq!(code, Some(1), "did not exit: {script:?}");
        assert_eq!(stdout, "", "trailing output after fatal: {script:?}");
    }
}

/// A compound's own failed redirection is the wrapper's fresh failure, not
/// an inherited one: still fatal under errexit (GNU rc 1).
#[test]
fn failed_redirection_on_compound_still_exits() {
    let dir = std::env::temp_dir().join(format!("rubash-issue507-redir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let script = format!("set -e; {{ :; }} > {}/missing/x; echo R\n", dir.display());
    let (code, stdout) = run_stdin(&script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
}

/// errexit itself still governs: `set +e` keeps everything running, and the
/// pass-through shapes behave identically without `set -e`.
#[test]
fn without_errexit_every_shape_continues() {
    let cases: [&str; 5] = [
        "if true; then false && true; fi; echo R\n",
        "if true; then false; fi; echo R\n",
        "for i in 1; do false; done; echo R\n",
        "{ false; }; echo R\n",
        "true && false; echo R\n",
    ];
    for script in cases {
        let (code, stdout) = run_stdin(script);
        assert_eq!(code, Some(0), "wrong rc without set -e: {script:?}");
        assert_eq!(stdout, "R\n", "wrong output without set -e: {script:?}");
    }
}
