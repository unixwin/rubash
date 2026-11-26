//! Issue rubash#520 — `trap -p ERR` inside a function must not show the
//! inherited ERR trap.
//!
//! GNU execute_cmd.c:5279-5286 (execute_function): without
//! error_trace_mode (`set -E`) the inherited ERR trap is REMOVED for the
//! function body (`restore_default_signal (ERROR_TRAP)`), with an
//! unwind-protect restoring it at exit. The Bash manual's set -E wording
//! — "any trap on ERR is inherited by shell functions" — cuts both ways:
//! unset, the function's trap context is empty, so `trap -p ERR` prints
//! nothing. The firing behavior was already correct in rubash (the
//! caller-level was_error_trap check fires OUT once after f returns);
//! only the display disagreed with the effective state. Rubash now saves
//! the ERR action at function entry, clears it, and restores it on exit
//! (uw_maybe_set_error_trap semantics: a trap the body set itself wins).
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (script-file probes, 2026-10-12).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue repro: `trap -p ERR` inside f prints nothing; `false` still
/// fires the outer ERR trap once and rc propagates (GNU: `---`/`OUT`/
/// `rc=1`, stderr empty).
#[test]
fn trap_p_err_inside_function_is_empty() {
    let (stdout, stderr, code) =
        rubash("trap \"echo OUT\" ERR; f() { trap -p ERR; echo ---; false; }; f; echo rc=$?");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "---\nOUT\nrc=1\n");
    assert_eq!(code, Some(0));
}

/// With `set -E` the ERR trap IS inherited, so the display shows it.
#[test]
fn trap_p_err_with_errtrace_shows_the_trap() {
    let (stdout, stderr, _code) = rubash("set -E; trap \"echo OUT\" ERR; f() { trap -p ERR; }; f");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "trap -- 'echo OUT' ERR\n");
}

/// The ERR trap comes back after the function returns (GNU
/// uw_maybe_set_error_trap): a later failure still fires it.
#[test]
fn err_trap_survives_the_function_boundary() {
    let (stdout, stderr, _code) =
        rubash("trap \"echo OUT\" ERR; f() { trap -p ERR; }; f; false; echo rc=$?");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "OUT\nrc=1\n");
}

/// An ERR trap the body set itself is its own context (GNU: the body's
/// trap fires for the body's failure and persists after return).
/// Expectation corrected at the 2026-10-23 bulk integration: GNU bash
/// 5.3.20 fires three times (the body's `false`, the failed call after
/// return, and the final top-level `false` — the body-set trap is the
/// visible one for all three, rubash#1163); the earlier two-fire
/// expectation came from a WSL GNU 5.3.0 check and pre-dates the
/// function-local ERR trap fix.
#[test]
fn body_set_err_trap_is_own_context() {
    let (stdout, stderr, _code) =
        rubash("trap \"echo OUTER\" ERR; f() { trap \"echo INNER\" ERR; false; }; f; false");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "INNER\nINNER\nINNER\n");
}

/// The DEBUG trap inheritance boundary is untouched: the call's own
/// simple-command fire plus the function-entry fire still happen (GNU
/// probe: `trap 'echo D' DEBUG; f() { :; }; f; echo done` prints
/// `D D done`).
#[test]
fn debug_trap_function_boundary_unchanged() {
    let (stdout, stderr, _code) = rubash("trap 'echo D' DEBUG; f() { :; }; f; echo done");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "D\nD\ndone\n");
}
