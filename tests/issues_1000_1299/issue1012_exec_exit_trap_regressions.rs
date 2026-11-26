//! rubash#1012: `trap ... EXIT` must not fire on a successful `exec`.
//!
//! GNU ground truth (bash 5.3.20, script-file mode, byte-compared):
//! builtins/exec.def exec_builtin hands the resolved command to
//! execute_cmd.c:6126 shell_execve, which replaces the shell image on
//! success — the exit path (shell.c:985 exit_shell -> run_exit_trap) is
//! never taken, so the EXIT trap does not run. trap.c:1440
//! reset_or_restore_signal_handlers clears SIG_TRAPPED for the exit trap
//! (keeping the string) BEFORE shell_execve, so a post-reset execve
//! failure (slash operand, e.g. ENOENT) also exits without the trap.
//! Only the pre-reset search failure (command == 0 -> sh_notfound ->
//! failed_exec -> exit_shell, exec.def:151-169) keeps it.
//!
//! Rubash cannot execve, so it spawns the replacement and unwinds with its
//! status; that unwind used to run the EXIT trap unconditionally.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_script(body: &str) -> (String, String, Option<i32>) {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "rubash-1012-{}.sh",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    {
        let mut file = std::fs::File::create(&path).expect("temp script");
        // LF only: the repo pins *.sh eol=lf (.gitattributes); a CR would
        // corrupt the baseline the same way it corrupts WSL runs.
        file.write_all(body.as_bytes()).expect("write script");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue repro: successful `exec` replaces the image, no exit path,
/// no EXIT trap (GNU: `REPLACED` only).
#[test]
fn exec_success_skips_exit_trap() {
    let (stdout, stderr, rc) = run_script("trap \"echo TRAPPED\" EXIT; exec echo REPLACED\n");
    assert_eq!(stdout, "REPLACED\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// A successful `exec` of a command that exits non-zero is still a
/// replacement: no trap, and the status is the program's (GNU: empty, 1).
#[test]
fn exec_success_nonzero_status_skips_exit_trap() {
    let (stdout, stderr, rc) = run_script("trap \"echo TRAPPED\" EXIT; exec false\n");
    assert_eq!(stdout, "", "stderr: {stderr:?}");
    assert_eq!(rc, Some(1));
}

/// Pre-reset failure (PATH search fails, no slash): sh_notfound ->
/// failed_exec -> exit_shell WITH the trap still set (GNU: `not found`
/// diagnostic, then TRAPPED, 127).
#[test]
fn exec_search_failure_keeps_exit_trap() {
    let (stdout, stderr, rc) = run_script("trap \"echo TRAPPED\" EXIT; exec no-such-cmd-1012\n");
    assert_eq!(stdout, "TRAPPED\n", "stderr: {stderr:?}");
    assert!(stderr.contains("not found"), "stderr: {stderr:?}");
    assert_eq!(rc, Some(127));
}

/// Post-reset failure (slash operand bypasses search; shell_execve fails
/// after reset_signal_handlers cleared the trap flag): no trap (GNU:
/// `No such file or directory`, no TRAPPED, 127).
#[test]
fn exec_slash_failure_skips_exit_trap() {
    let (stdout, stderr, rc) =
        run_script("trap \"echo TRAPPED\" EXIT; exec /nonexistent-xyz-1012\n");
    assert_eq!(stdout, "", "stderr: {stderr:?}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(127));
}

/// Control: an ordinary exit still runs the EXIT trap.
#[test]
fn normal_exit_keeps_exit_trap() {
    let (stdout, stderr, rc) = run_script("trap \"echo TRAPPED\" EXIT; echo DONE\n");
    assert_eq!(stdout, "DONE\nTRAPPED\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// A subshell's exec replaces only the subshell: its own EXIT trap is
/// skipped, but the parent continues and its trap still fires at parent
/// exit (GNU: `SUB-REPLACED`, `PARENT-AFTER`, `PARENT-TRAPPED`).
#[test]
fn subshell_exec_skips_subshell_trap_keeps_parent_trap() {
    let (stdout, stderr, rc) = run_script(
        "trap \"echo PARENT-TRAPPED\" EXIT; (trap \"echo SUB-TRAPPED\" EXIT; exec echo SUB-REPLACED); echo PARENT-AFTER\n",
    );
    assert_eq!(
        stdout, "SUB-REPLACED\nPARENT-AFTER\nPARENT-TRAPPED\n",
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

/// Nested subshells: the innermost image is replaced (no INNER trap) while
/// each surviving outer frame still exits through its own trap.
#[test]
fn nested_subshell_exec_keeps_outer_trap() {
    let (stdout, stderr, rc) =
        run_script("trap \"echo OUTER\" EXIT; ( trap \"echo INNER\" EXIT; exec echo HI )\n");
    assert_eq!(stdout, "HI\nOUTER\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// `exec` inside a function replaces the whole shell (functions run in the
/// current shell, no fork): nothing after it runs, no trap (GNU: `F`).
#[test]
fn function_exec_replaces_shell_without_trap() {
    let (stdout, stderr, rc) =
        run_script("trap \"echo TRAPPED\" EXIT; f() { exec echo F; }; f; echo AFTER\n");
    assert_eq!(stdout, "F\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}
