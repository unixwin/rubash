//! Issue rubash#439 — the ERR trap must not fire for a word/assignment
//! expansion error.
//!
//! GNU fires the ERR trap only for a simple command that EXECUTED and
//! failed (run_error_trap callers, execute_cmd.c:773/874/1000/1166, sit
//! below expansion in execute_simple_command). A word/assignment expansion
//! error abandons the command via exp_jump_to_top_level(DISCARD)
//! (subst.c expand_wdesc_error) BEFORE execution, so the trap never fires:
//!
//! ```text
//! trap 'echo ERR-FIRED' ERR
//! x=${!bad}
//! echo after
//! ```
//!
//! GNU prints `bad: invalid indirect expansion` and `after` — no ERR-FIRED.
//! rubash used to print an extra ERR-FIRED line.
//!
//! Failure classes that MUST keep firing the ERR trap are pinned too: a
//! plain failing command, the set -E errtrace function inheritance, the
//! EXIT trap's independence, and context classes GNU traps (subshell death,
//! arithmetic failure, redirection failure).
//!
//! Expected outputs byte-verified against Git for Windows GNU Bash
//! 5.2.37 (msys, D:\Git\usr\bin\bash — oracle probes 2026-10-09).

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

/// #439: the indirect expansion error reports the diagnostic and continues
/// to the next line WITHOUT firing the ERR trap.
#[test]
fn indirect_expansion_error_does_not_fire_err_trap() {
    let (stdout, stderr, code) = rubash("trap 'echo ERR-FIRED' ERR\nx=${!bad}\necho after");
    assert!(stderr.contains("invalid indirect expansion"), "{stderr:?}");
    assert_eq!(stdout, "after\n");
    assert_eq!(code, Some(0));
}

/// #439: same suppression under `set -E` (errtrace changes function
/// inheritance, not the expansion-error class).
#[test]
fn indirect_expansion_error_does_not_fire_err_trap_under_errtrace() {
    let (stdout, stderr, code) = rubash("set -E\ntrap 'echo ERR-FIRED' ERR\nx=${!bad}\necho after");
    assert!(stderr.contains("invalid indirect expansion"), "{stderr:?}");
    assert_eq!(stdout, "after\n");
    assert_eq!(code, Some(0));
}

/// #439: a word expansion error (`echo ${!bad}`) is the same class — no ERR
/// trap; the next line runs and the script exits with its status (GNU 5.2.37
/// script probe: `after`, rc 0).
#[test]
fn word_expansion_error_does_not_fire_err_trap() {
    let (stdout, stderr, code) = rubash("trap 'echo E' ERR\necho ${!bad}\necho after");
    assert!(stderr.contains("invalid indirect expansion"), "{stderr:?}");
    assert_eq!(stdout, "after\n");
    assert_eq!(code, Some(0));
}

/// #439: an unset nameref-style indirect source reports the same way.
#[test]
fn unset_nameref_indirect_error_does_not_fire_err_trap() {
    let (stdout, stderr, code) = rubash("trap 'echo E' ERR\nunset -n ref\nx=${!ref}\necho after");
    assert!(stderr.contains("invalid indirect expansion"), "{stderr:?}");
    assert_eq!(stdout, "after\n");
    assert_eq!(code, Some(0));
}

/// A failing simple command MUST still fire the ERR trap (GNU: E, after).
#[test]
fn plain_command_failure_still_fires_err_trap() {
    let (stdout, stderr, code) = rubash("trap 'echo E' ERR\nfalse\necho after");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "E\nafter\n");
    assert_eq!(code, Some(0));
}

/// `set -E` errtrace: a failure inside a function fires the trap at both
/// depths (GNU: E, E, after).
#[test]
fn errtrace_function_failure_still_fires_err_trap() {
    let (stdout, stderr, code) = rubash("set -E\ntrap 'echo E' ERR\nf() { false; }\nf\necho after");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "E\nE\nafter\n");
    assert_eq!(code, Some(0));
}

/// A command substitution failure in an assignment is NOT an expansion
/// error: the inner command ran, so the ERR trap fires (GNU: E, after).
#[test]
fn command_substitution_failure_still_fires_err_trap() {
    let (stdout, stderr, code) = rubash("trap 'echo E' ERR\nx=$(false)\necho after");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "E\nafter\n");
    assert_eq!(code, Some(0));
}

/// A subshell dying from an internal expansion error is judged by its EXIT
/// STATUS in the parent: the ERR trap fires (GNU: E, after).
#[test]
fn subshell_expansion_death_still_fires_err_trap() {
    let (stdout, stderr, code) = rubash("trap 'echo E' ERR\n(x=${!bad})\necho after");
    assert!(stderr.contains("invalid indirect expansion"), "{stderr:?}");
    assert_eq!(stdout, "E\nafter\n");
    assert_eq!(code, Some(0));
}

/// An arithmetic evaluation error is a failing `(( ))` command, not a word
/// expansion error: the ERR trap fires (GNU: E, after).
#[test]
fn arithmetic_failure_still_fires_err_trap() {
    let (stdout, stderr, code) = rubash("trap 'echo E' ERR\n(( 1/0 ))\necho after");
    assert!(stderr.contains("division by 0"), "{stderr:?}");
    assert_eq!(stdout, "E\nafter\n");
    assert_eq!(code, Some(0));
}

/// The EXIT trap is untouched by any of this (GNU: hi, BYE).
#[test]
fn exit_trap_unaffected() {
    let (stdout, stderr, code) = rubash("trap 'echo BYE' EXIT\necho hi");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hi\nBYE\n");
    assert_eq!(code, Some(0));
}

/// The expansion-error mark is per command: a failure on a LATER command
/// still fires (GNU: E, after).
#[test]
fn err_trap_fires_for_later_command_after_expansion_error() {
    let (stdout, stderr, code) = rubash("trap 'echo E' ERR\nx=${!bad}\nfalse\necho after");
    assert!(stderr.contains("invalid indirect expansion"), "{stderr:?}");
    assert_eq!(stdout, "E\nafter\n");
    assert_eq!(code, Some(0));
}
