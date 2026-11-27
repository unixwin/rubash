//! rubash#3409 — the EXIT trap fires inside a pipeline element, so
//! `trap 'echo bye' EXIT; type ls | cat` prints `bye` twice: once
//! when the pipeline element tears down, once at the main shell's exit.
//!
//! Root cause: `execute_builtin_pipeline_stage`
//! (src/executor/pipeline_stages.rs) clones a subshell executor for the
//! builtin element and runs the subshell's EXIT trap at teardown
//! (`run_exit_trap_for_status`), but — unlike its compound and
//! function siblings, which both call `reset_for_subshell_child` at
//! entry — it never resets the inherited trap table, so the parent's
//! live EXIT action fires in the element. GNU disarms the EXIT trap in
//! every forked pipeline-element child: trap.c
//! `reset_or_restore_signal_handlers` clears SIG_TRAPPED for
//! EXIT_TRAP, called via `reset_signal_handlers()` from
//! execute_cmd.c:1670 (`execute_in_subshell`) and execute_cmd.c:4833
//! (the already-forked simple-command child that runs a builtin or
//! function element, `execute_subshell_builtin_or_function`). A trap
//! set INSIDE the element still fires at its exit (GNU keeps the
//! trap string; `set_trap` re-arms), so only the inherited action is
//! suppressed.
//!
//! Expectations verified against GNU bash 5.3.20 (oracle, `-c`
//! scripts, 2026-11-14): the parent's EXIT trap prints exactly once
//! in every shape below; a trap set inside a `( )` pipeline element
//! fires once at the element's exit and the parent's once at shell exit.

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

/// Count the EXIT trap's own line: the defect prints it twice.
fn bye_lines(stdout: &str) -> usize {
    stdout.lines().filter(|line| *line == "bye").count()
}

#[test]
fn issue3409_exit_trap_fires_once_for_builtin_pipeline_element() {
    let (stdout, _stderr, code) = rubash("trap 'echo bye' EXIT\ntype echo | cat");
    assert_eq!(bye_lines(&stdout), 1, "stdout: {stdout}");
    assert!(
        stdout.contains("echo is a shell builtin"),
        "stdout: {stdout}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue3409_exit_trap_fires_once_for_builtin_pipeline_tail() {
    let (stdout, _stderr, code) = rubash("trap 'echo bye' EXIT\necho hi | type cat");
    assert_eq!(bye_lines(&stdout), 1, "stdout: {stdout}");
    assert!(stdout.contains("cat is /usr/bin/cat"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3409_exit_trap_fires_once_for_external_pipeline() {
    let (stdout, _stderr, code) = rubash("trap 'echo bye' EXIT\necho hi | cat");
    assert_eq!(stdout, "hi\nbye\n", "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3409_exit_trap_fires_once_for_paren_subshell() {
    let (stdout, _stderr, code) = rubash("trap 'echo bye' EXIT\n(echo in)");
    assert_eq!(stdout, "in\nbye\n", "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3409_exit_trap_fires_once_for_command_substitution() {
    let (stdout, _stderr, code) = rubash("trap 'echo bye' EXIT\nx=$(echo hi); echo x=$x");
    assert_eq!(stdout, "x=hi\nbye\n", "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3409_trap_set_inside_pipeline_element_fires_at_element_exit() {
    let (stdout, _stderr, code) =
        rubash("trap 'echo bye' EXIT\n(trap 'echo inner' EXIT; echo x) | cat");
    assert_eq!(stdout, "x\ninner\nbye\n", "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3409_exit_trap_fires_once_under_lastpipe() {
    let (stdout, _stderr, code) =
        rubash("shopt -s lastpipe\ntrap 'echo bye' EXIT\ntype echo | cat\necho after");
    assert_eq!(bye_lines(&stdout), 1, "stdout: {stdout}");
    assert!(
        stdout.contains("echo is a shell builtin"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("after\n"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}
