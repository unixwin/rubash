//! Issue rubash#702 — two errexit / trap-inheritance divergences from GNU
//! bash, both deterministic.
//!
//! R4.1: under `set -e`, an `until` loop whose body starts with `break` or
//! `continue` (directly or as the first command of a group) attributed the
//! loop CONDITION's failing status to that body command. The dispatcher's
//! post-command errexit / ERR-trap gate reads the shell's status, which the
//! condition left non-zero (it runs with errexit suppressed), and the
//! `break`/`continue` builtin unwound without establishing its own status.
//! GNU's break.def/continue.def return EXECUTION_SUCCESS, so
//! `until false; do break; done` reaches the following command and leaves
//! `$? = 0`.
//!
//! R4.2: `set -E` (errtrace) is not propagated into forked `( )` subshells,
//! so the inherited ERR trap never fired for the subshell body's failing
//! commands. GNU trap.c:1588 reset_or_restore_signal_handlers keeps the ERR
//! trap in a subshell child when error_trace_mode is set, exactly as it
//! keeps DEBUG/RETURN under functrace.
//!
//! Every expectation below is the literal GNU bash 5.3.0 output (the
//! `D:/Git/bin/bash.exe` reference build).

use std::process::Command;

fn run(source: &str) -> (String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", source])
        .env_remove("BASH_ENV")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

// ---------------------------------------------------------------------------
// R4.1 — `until` body starting with break/continue
// ---------------------------------------------------------------------------

/// The issue repro: the body's `break` must not abort on the condition's
/// failing status.
#[test]
fn until_break_reaches_following_command() {
    let (out, code) = run("set -e; until false; do break; done; echo REACHED");
    assert_eq!(out, "REACHED\n");
    assert_eq!(code, 0);
}

/// The loop's own status is 0 after a break (the stray condition status must
/// not become `$?`).
#[test]
fn until_break_leaves_zero_status() {
    let (out, code) = run("until false; do break; done; echo rc=$?");
    assert_eq!(out, "rc=0\n");
    assert_eq!(code, 0);
}

/// `while` was never affected and must stay correct.
#[test]
fn while_break_stays_correct() {
    let (out, code) = run("set -e; while true; do break; done; echo REACHED");
    assert_eq!(out, "REACHED\n");
    assert_eq!(code, 0);
}

/// `for` was never affected and must stay correct.
#[test]
fn for_break_stays_correct() {
    let (out, code) = run("set -e; for i in 1; do break; done; echo REACHED");
    assert_eq!(out, "REACHED\n");
    assert_eq!(code, 0);
}

/// A command before the break already overwrote the condition status, so
/// this form was already correct and must stay correct.
#[test]
fn until_body_with_leading_noop_before_break_stays_correct() {
    let (out, code) = run("set -e; until false; do :; break; done; echo REACHED");
    assert_eq!(out, "REACHED\n");
    assert_eq!(code, 0);
}

/// The characterization's group case: a brace group whose first command is
/// `break` is still the body's first executed command.
#[test]
fn until_body_group_starting_with_break() {
    let (out, code) = run("set -e; until false; do { break; }; done; echo REACHED");
    assert_eq!(out, "REACHED\n");
    assert_eq!(code, 0);
}

/// `continue` aborts a loop only when the condition's status leaks; the loop
/// must run to its natural termination and reach the following command.
#[test]
fn until_continue_reaches_loop_termination() {
    let (out, code) = run("set -e; i=0; until [ $((++i)) -ge 2 ]; do continue; done; echo REACHED");
    assert_eq!(out, "REACHED\n");
    assert_eq!(code, 0);
}

/// The condition's status must not leak into the loop's exit status: the
/// condition failed with 7, but the break ends the loop successfully.
#[test]
fn until_condition_status_does_not_leak() {
    let (_out, code) = run("set -e; until (exit 7); do break; done");
    assert_eq!(code, 0);
}

/// The stray attribution also fired the ERR trap with the condition's
/// status; the body's first `break` must not fire it.
#[test]
fn until_break_does_not_fire_err_trap() {
    let (out, code) =
        run("set -e; trap 'echo ERR:$?' ERR; until false; do break; done; echo AFTER");
    assert_eq!(out, "AFTER\n");
    assert_eq!(code, 0);
}

// ---------------------------------------------------------------------------
// R4.2 — `set -E` propagation into `( )` subshells
// ---------------------------------------------------------------------------

/// Without errtrace the subshell does not inherit the ERR trap: only the
/// failing subshell command fires it in the parent.
#[test]
fn err_trap_not_inherited_without_errtrace() {
    let (out, code) = run("trap 'echo ERR' ERR; ( false ); echo done");
    assert_eq!(out, "ERR\ndone\n");
    assert_eq!(code, 0);
}

/// With `set -E` the inherited trap fires for the inner `false` AND for the
/// failing subshell command.
#[test]
fn errtrace_inherits_err_trap_into_subshell() {
    let (out, code) = run("set -E; trap 'echo ERR' ERR; ( false ); echo done");
    assert_eq!(out, "ERR\nERR\ndone\n");
    assert_eq!(code, 0);
}

/// The inherited trap fires for a failing body command even when the
/// subshell itself ends successfully.
#[test]
fn errtrace_fires_for_inner_failure_only() {
    let (out, code) = run("set -E; trap 'echo ERR' ERR; ( false; true ); echo done");
    assert_eq!(out, "ERR\ndone\n");
    assert_eq!(code, 0);
}

/// The issue's `( f )` case: 3 ERR — the function body's failure, the failed
/// function call inside the subshell, and the failed subshell command.
#[test]
fn errtrace_inherits_err_trap_for_function_in_subshell() {
    let (out, code) = run("set -E; trap 'echo ERR' ERR; f(){ false; }; ( f ); echo done");
    assert_eq!(out, "ERR\nERR\nERR\ndone\n");
    assert_eq!(code, 0);
}

/// The issue's command-substitution example: the `(false)` inside the
/// substitution inherits `-E` set inside the substitution.
#[test]
fn errtrace_inherits_err_trap_in_comsub_subshell() {
    let (out, code) = run("o=$(set -E; trap 'echo E' ERR; (false)); echo \"[$o]\"");
    assert_eq!(out, "[E\nE]\n");
    assert_eq!(code, 0);
}

/// The subshell keeps listing the inherited ERR trap even without errtrace
/// (GNU reset does not free the trap string).
#[test]
fn subshell_still_lists_inherited_err_trap() {
    let (out, code) = run("trap 'echo ERR' ERR; ( trap -p ERR ); echo done");
    assert_eq!(out, "trap -- 'echo ERR' ERR\ndone\n");
    assert_eq!(code, 0);
}
