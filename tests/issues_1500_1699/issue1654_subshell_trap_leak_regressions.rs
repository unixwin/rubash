//! rubash#1654 — a synchronous subshell's trap disposition leaked
//! into the parent: `(trap '' TERM; kill -s TERM $$; echo sub);
//! echo parent=$?` printed `parent=0` because the subshell's SIG_IGN
//! was the process-wide kernel disposition when the kill builtin
//! raise(2)d the parent-addressed signal. In GNU the subshell is a
//! separate process; `$$` names the parent shell, whose OWN
//! disposition judges the delivery: untrapped -> the parent process
//! dies by SIGTERM after the subshell body completes (a parent shell
//! would read wait status 143; no parent line),
//! trapped -> the parent's trap runs once the foreground subshell
//! completes and the list continues, ignored -> the delivery drops.
//! The fix routes own-pid deliveries made from a flat subshell layer
//! to the depth-0 boundary dispatch (ANCESTOR_SIGNALS in kill.rs),
//! which judges them against the restored top-level trap table.
//!
//! Expectations verified against GNU bash 5.3.20 (oracle, script
//! files, 3/3 runs each).

#![cfg(unix)]

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

#[test]
fn issue1654_subshell_ignore_does_not_shield_parent_from_term() {
    let (stdout, _stderr, code) =
        rubash("(trap '' TERM; kill -s TERM $$; echo sub); echo parent=$?");
    assert_eq!(stdout, "sub\n", "stdout: {stdout}");
    // Process-level expectation (battery READJUDICATION, adj-signals
    // §issue1654): GNU dies BY SIGTERM in this shape (subprocess
    // returncode -15), exactly like the control below — 143 is the
    // shell-layer wait status, not a process exit code, so
    // ExitStatus::code() is None. The original Some(143) pin
    // contradicted the oracle.
    assert_eq!(code, None);
}

#[test]
fn issue1654_subshell_catch_trap_does_not_consume_parent_delivery() {
    // The subshell's own TERM trap must NOT fire for a $$-addressed
    // delivery (GNU: the signal belongs to the parent process, which
    // is untrapped and dies).
    let (stdout, _stderr, code) =
        rubash("(trap 'echo got' TERM; kill -s TERM $$; echo subalive); echo parentalive");
    assert_eq!(stdout, "subalive\n", "stdout: {stdout}");
    // Signal death, as in the sibling above (adj-signals §issue1654):
    // GNU returncode -15, so code() is None, not Some(143).
    assert_eq!(code, None);
}

#[test]
fn issue1654_parent_trap_runs_after_subshell_completes() {
    let (stdout, _stderr, code) = rubash(
        "trap 'echo PARENT_TRAP' TERM\n\
         (trap '' TERM; kill -s TERM $$; echo sub); echo parent=$?",
    );
    assert_eq!(stdout, "sub\nPARENT_TRAP\nparent=0\n", "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1654_parent_ignore_drops_the_delivery() {
    let (stdout, _stderr, code) =
        rubash("trap '' TERM; (kill -s TERM $$; echo sub); echo parent=$?");
    assert_eq!(stdout, "sub\nparent=0\n", "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1654_nested_frames_continue_then_parent_dies() {
    // GNU (file-capture verified): the inner body completes, the
    // outer subshell's remaining commands still run (it is its own
    // process), and the death lands when control returns to the
    // top-level shell — `parent` never prints.
    let (stdout, _stderr, code) =
        rubash("(( trap '' TERM; kill -s TERM $$; echo sub2 ); echo mid); echo parent");
    assert_eq!(stdout, "sub2\nmid\n", "stdout: {stdout}");
    // Signal death (adj-signals §issue1654): GNU returncode -15, so
    // code() is None, not Some(143).
    assert_eq!(code, None);
}

#[test]
fn issue1654_exit_trap_runs_on_parent_signal_death() {
    let (stdout, _stderr, code) = rubash(
        "trap 'echo EXITED' EXIT\n\
         (trap '' TERM; kill -s TERM $$; echo sub); echo parent=$?",
    );
    assert_eq!(stdout, "sub\nEXITED\n", "stdout: {stdout}");
    // The EXIT trap runs, then the signal death lands (adj-signals
    // §issue1654): GNU returncode -15, so code() is None, not
    // Some(143).
    assert_eq!(code, None);
}

#[test]
fn issue1654_plain_self_kill_control_unchanged() {
    let (stdout, _stderr, code) = rubash("kill -s TERM $$; echo survived");
    assert_eq!(stdout, "", "stdout: {stdout}");
    // Process-level expectation (GATE-READJUDICATION A2): the shell
    // process dies BY SIGTERM itself — under the same probe GNU's
    // returncode is -15 (signal death), and Rust's
    // ExitStatus::code() is None for a signal death. 143 is the
    // shell-layer wait status (128+15), not a process exit code;
    // the original pin's Some(143) could only pass if rubash
    // synthesized exit(143), which would diverge from GNU's
    // signal-death shape. (The battery readjudication later
    // extended this same correction to the four deferred-death
    // sibling pins in this file: GNU dies by SIGTERM there too,
    // so they now pin None as well.)
    assert_eq!(code, None);
}
