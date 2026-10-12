//! rubash#545: process-substitution fd lifetime and `[[ ]]` operand xtrace.
//!
//! Three script-observable cells (WSL GNU 5.3.0(1) probes, 2026-10-12):
//!
//! cell 1 — `exec <&-` then `read`: fd 0 is closed, so `read` reports
//!   `read: 0: read error: Bad file descriptor`, rc 1, and the script
//!   CONTINUES (`echo after=$?` prints 1). A read whose stdin is
//!   re-pointed by its own redirect (`read x <<< hi`) succeeds.
//!
//! cell 2 — a process substitution stored in an array dies with its
//!   creating command: `a=(<(echo p9)); cat "${a[0]}"` fails ENOENT rc 1
//!   (GNU reaps the /dev/fd/N path when the assignment finishes; the
//!   stored string is a dead path). The substitution body ran exactly once
//!   (at the assignment).
//!
//! cell 3 — a `[[ ]]` process-substitution operand is realized and
//!   traced: `set -x; [[ -r <(echo m) ]]` emits the `+ [[ -r /dev/fd/N ]]`
//!   term line (GNU xtrace_print_cond_term prints the realized path) and
//!   runs the body. GNU's body trace order is scheduler-dependent; rubash
//!   collects the body synchronously, so only line PRESENCE is asserted.

use std::process::Command;

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("__RUBASH_TRAP_ORIG_IGN")
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
fn read_after_exec_closed_stdin_reports_ebadf_and_continues() {
    let (stdout, stderr, rc) = run("exec <&-\nread x\necho after=$?");
    assert_eq!(stdout, "after=1\n");
    assert!(
        stderr.contains("read: 0: read error: Bad file descriptor"),
        "stderr: {stderr:?}"
    );
    // GNU: the failed read does not abort the script; the shell itself
    // finishes normally.
    assert_eq!(rc, Some(0));
}

#[test]
fn read_with_own_redirect_after_exec_closed_stdin_succeeds() {
    let (stdout, _stderr, rc) = run("exec <&-\nread x <<< hi\necho got=[$x] rc=$?");
    assert_eq!(stdout, "got=[hi] rc=0\n");
    assert_eq!(rc, Some(0));
}

#[test]
fn array_stored_procsub_path_is_dead_after_the_assignment() {
    let (stdout, stderr, rc) = run("a=(<(echo p9)); cat \"${a[0]}\"; echo rc=$?");
    // The body ran at the assignment; the stored /dev/fd/N string names a
    // dead path for every later command (GNU reaps the substitution with
    // the creating command).
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout:?}");
    assert!(
        stderr.contains("/dev/fd/") && stderr.contains("No such file or directory"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn array_stored_procsub_body_runs_exactly_once() {
    let (stdout, _stderr, _rc) =
        run("a=(<(echo once)); cat \"${a[0]}\" >/dev/null; cat \"${a[0]}\" >/dev/null; echo done");
    assert_eq!(stdout, "done\n");
    // No visible body output may reach stdout: the substitution produced
    // `once` only inside the dead fd, never on the script's stdout.
    assert!(!stdout.contains("once"));
}

#[test]
fn conditional_procsub_operand_is_traced_with_the_realized_path() {
    let (stdout, stderr, rc) = run("set -x; [[ -r <(echo m) ]]; echo rc=$?");
    assert_eq!(stdout, "rc=0\n");
    // The term line names the realized /dev/fd/N path (GNU
    // xtrace_print_cond_term) and the body fires (its own `++ echo m`
    // trace). Line ORDER is scheduler-dependent in GNU and deterministic
    // (body first) in rubash — assert presence, not order.
    let realized = stderr
        .lines()
        .find(|line| line.contains("[[ -r /dev/fd/"))
        .expect("realized [[ ]] term trace line");
    assert!(realized.trim_end().ends_with("]]"), "line: {realized:?}");
    assert!(stderr.contains("++ echo m"), "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

#[test]
fn conditional_procsub_operand_without_xtrace_still_runs_the_body() {
    // Realization is not gated on `set -x`: GNU runs the substitution for
    // `[[ -r <(echo m) ]]` regardless of tracing.
    let (stdout, stderr, rc) = run("[[ -r <(echo side-effect) ]]; echo rc=$?");
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
}
