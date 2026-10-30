//! rubash#2125 — a `{var}` fd_var redirection failure on a POSIX
//! special builtin must exit a non-interactive POSIX-mode shell;
//! rubash reported the error, set status 1, and kept executing.
//!
//! GNU: the failure surfaces inside do_redirections through
//! redir_varassign (redir.c:1133-1166), whose EX_REDIRFAIL return
//! (execute_cmd.c:5613) takes the execute_cmd.c:4865-4875 conversion —
//! `posixly_correct && builtin_is_special && interactive_shell == 0`
//! → immediate jump_to_top_level (ERREXIT) at status 1. Tested
//! contexts (`||`, if, `!`) do not contain it, the ERR trap never
//! runs, the EXIT trap still runs. Non-special commands (ordinary
//! builtins, disk commands), redirect-only commands and the
//! `command` re-dispatch continue at status 1, as does every face in
//! non-POSIX mode.
//!
//! Rubash applies fd_var redirects in apply_dynamic_fd_var_redirects
//! (and inline in bare `exec`'s stdio-only path), which swallowed the
//! IoError into a bool the #2082 conversion sites never see. The fix
//! converts at each applier call site under the #2082 gate
//! (posix_special_redirect_failure_exits). The operand-bearing
//! `exec` face additionally needed its failure kept OUT of
//! finish_exec_replacement: a do_redirections failure precedes
//! exec_builtin's search/execve, so exec.def's failed_exec exit never
//! applies to it (non-POSIX `exec cmd {fd}>/bad` continues, and
//! `execfail` does not govern the POSIX jump either).
//!
//! All expectations measured against GNU bash 5.3.20 with script
//! files (self-compiled oracle, 2026-10-30). The failing target is a
//! path inside a nonexistent directory, so the open fails with
//! ENOENT deterministically on every platform.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2125-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg(&case)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const BAD: &str = "/nonexistent-dir/x";

/// POSIX + fd_var failure on a special builtin: diagnostic, exit 1,
/// nothing after the failing command runs.
fn assert_posix_fdvar_exits(tag: &str, body: &str) {
    assert_posix_fdvar_exits_target(tag, body, BAD);
}

fn assert_posix_fdvar_exits_target(tag: &str, body: &str, target: &str) {
    let script = format!("set -o posix\n{body}\n");
    let (stdout, stderr, code) = run(tag, &script);
    assert!(
        !stdout.contains("AFTER"),
        "{tag}: stdout must not reach AFTER: {stdout}"
    );
    assert!(
        stderr.contains(target),
        "{tag}: stderr must name the target: {stderr}"
    );
    assert_eq!(code, Some(1), "{tag}: exit code (stderr: {stderr})");
}

/// The same shape outside POSIX mode: status 1, shell continues.
fn assert_nonposix_fdvar_continues(tag: &str, body: &str) {
    let (stdout, stderr, code) = run(tag, &format!("{body}\n"));
    assert!(
        stdout.contains("AFTER"),
        "{tag}: stdout must reach AFTER: {stdout}"
    );
    assert!(
        stderr.contains(BAD),
        "{tag}: stderr must name the target: {stderr}"
    );
    assert_eq!(code, Some(0), "{tag}: exit code (stderr: {stderr})");
}

// ---- POSIX fatal faces -------------------------------------------------

#[test]
fn issue2125_posix_bare_exec_fdvar_failure_exits() {
    assert_posix_fdvar_exits("exec-out", &format!("exec {{fd}}>{BAD}\necho AFTER"));
}

#[test]
fn issue2125_posix_bare_exec_fdvar_failure_not_contained_by_or() {
    assert_posix_fdvar_exits(
        "exec-or",
        &format!("exec {{fd}}>{BAD} || echo OR\necho AFTER"),
    );
}

#[test]
fn issue2125_posix_bare_exec_fdvar_input_failure_exits() {
    assert_posix_fdvar_exits_target(
        "exec-in",
        "exec {fd}</nonexistent-file-x\necho AFTER",
        "/nonexistent-file-x",
    );
}

#[test]
fn issue2125_posix_bare_exec_fdvar_readwrite_failure_exits() {
    assert_posix_fdvar_exits("exec-rw", &format!("exec {{fd}}<>{BAD}\necho AFTER"));
}

#[test]
fn issue2125_posix_export_fdvar_failure_exits() {
    assert_posix_fdvar_exits("export", &format!("export -p {{fd}}>{BAD}\necho AFTER"));
}

#[test]
fn issue2125_posix_exec_operand_fdvar_failure_exits() {
    assert_posix_fdvar_exits(
        "exec-operand",
        &format!("exec /bin/true {{fd}}>{BAD}\necho AFTER"),
    );
}

#[test]
fn issue2125_posix_exec_operand_fdvar_failure_not_contained_by_or() {
    assert_posix_fdvar_exits(
        "exec-operand-or",
        &format!("exec /bin/true {{fd}}>{BAD} || echo OR\necho AFTER"),
    );
}

#[test]
fn issue2125_posix_exec_operand_fdvar_failure_ignores_execfail() {
    // The jump is the EX_REDIRFAIL conversion, not exec.def's
    // failed_exec, so the execfail shopt does not contain it.
    assert_posix_fdvar_exits(
        "exec-execfail",
        &format!("shopt -s execfail\nexec /bin/true {{fd}}>{BAD}\necho AFTER"),
    );
}

#[test]
fn issue2125_posix_fdvar_failure_skips_err_trap_but_runs_exit_trap() {
    let script = format!(
        "set -o posix\ntrap 'echo ERRTRAP' ERR\ntrap 'echo EXITT' EXIT\nexec {{fd}}>{BAD}\necho AFTER\n"
    );
    let (stdout, stderr, code) = run("traps", &script);
    assert!(
        !stdout.contains("ERRTRAP"),
        "ERR trap must not run: {stdout}"
    );
    assert!(stdout.contains("EXITT"), "EXIT trap must run: {stdout}");
    assert!(!stdout.contains("AFTER"), "no AFTER: {stdout}");
    assert_eq!(code, Some(1), "exit code (stderr: {stderr})");
}

#[test]
fn issue2125_posix_fdvar_failure_inside_function_exits() {
    assert_posix_fdvar_exits(
        "in-func",
        &format!("f(){{ exec {{fd}}>{BAD}; echo IN-FUNC-AFTER; }}\nf\necho AFTER"),
    );
}

#[test]
fn issue2125_posix_fdvar_failure_under_time_exits() {
    let script = format!("set -o posix\ntime exec {{fd}}>{BAD}\necho AFTER\n");
    let (stdout, stderr, code) = run("time", &script);
    assert!(!stdout.contains("AFTER"), "no AFTER: {stdout}");
    assert!(
        stderr.contains("real"),
        "timing report precedes the exit: {stderr}"
    );
    assert_eq!(code, Some(1), "exit code (stderr: {stderr})");
}

// ---- POSIX continuing faces ---------------------------------------------

#[test]
fn issue2125_posix_fdvar_failure_on_plain_builtin_continues() {
    let script = format!("set -o posix\necho hi {{fd}}>{BAD}\necho \"AFTER rc=$?\"\n");
    let (stdout, stderr, code) = run("echo-builtin", &script);
    assert!(stdout.contains("AFTER rc=1"), "stdout: {stdout}");
    assert_eq!(code, Some(0), "exit code (stderr: {stderr})");
}

#[test]
fn issue2125_posix_fdvar_failure_on_external_continues() {
    let script = format!("set -o posix\nls {{fd}}>{BAD}\necho \"AFTER rc=$?\"\n");
    let (stdout, stderr, code) = run("external", &script);
    assert!(stdout.contains("AFTER rc=1"), "stdout: {stdout}");
    assert_eq!(code, Some(0), "exit code (stderr: {stderr})");
}

#[test]
fn issue2125_posix_command_exec_fdvar_failure_continues() {
    // The `command` re-dispatch never carries builtin_is_special.
    let script = format!("set -o posix\ncommand exec {{fd}}>{BAD}\necho AFTER\n");
    let (stdout, stderr, code) = run("command-exec", &script);
    assert!(stdout.contains("AFTER"), "stdout: {stdout}");
    assert_eq!(code, Some(0), "exit code (stderr: {stderr})");
}

#[test]
fn issue2125_posix_redirect_only_fdvar_failure_continues() {
    let script = format!("set -o posix\n{{fd}}>{BAD}\necho AFTER\n");
    let (stdout, stderr, code) = run("redir-only", &script);
    assert!(stdout.contains("AFTER"), "stdout: {stdout}");
    assert_eq!(code, Some(0), "exit code (stderr: {stderr})");
}

#[test]
fn issue2125_posix_fdvar_failure_in_comsub_is_contained() {
    let script = format!("set -o posix\nx=$(exec {{fd}}>{BAD})\necho \"AFTER rc=$?\"\n");
    let (stdout, stderr, code) = run("comsub", &script);
    assert!(stdout.contains("AFTER rc=1"), "stdout: {stdout}");
    assert_eq!(code, Some(0), "exit code (stderr: {stderr})");
}

// ---- Non-POSIX controls ---------------------------------------------------

#[test]
fn issue2125_nonposix_bare_exec_fdvar_failure_continues() {
    assert_nonposix_fdvar_continues("np-exec", &format!("exec {{fd}}>{BAD}\necho AFTER"));
}

#[test]
fn issue2125_nonposix_export_fdvar_failure_continues() {
    assert_nonposix_fdvar_continues("np-export", &format!("export -p {{fd}}>{BAD}\necho AFTER"));
}

#[test]
fn issue2125_nonposix_exec_operand_fdvar_failure_continues() {
    // Baseline rubash exited here even without POSIX mode: the
    // do_redirections failure was misread as a failed exec.
    assert_nonposix_fdvar_continues(
        "np-exec-operand",
        &format!("exec /bin/true {{fd}}>{BAD}\necho AFTER"),
    );
}

#[test]
fn issue2125_nonposix_fdvar_failure_leaves_variable_unbound() {
    let script = format!("exec {{fd}}>{BAD}\necho \"AFTER fd=${{fd-UNSET}}\"\n");
    let (stdout, stderr, code) = run("np-undo", &script);
    assert!(stdout.contains("AFTER fd=UNSET"), "stdout: {stdout}");
    assert_eq!(code, Some(0), "exit code (stderr: {stderr})");
}

// ---- Success guard --------------------------------------------------------

#[test]
fn issue2125_posix_fdvar_success_form_still_works() {
    let script =
        "set -o posix\nexec {fd}>./out.txt\necho hello >&$fd\nexec {fd}>&-\ncat ./out.txt\n";
    let (stdout, stderr, code) = run("success", script);
    assert_eq!(stdout, "hello\n", "stdout (stderr: {stderr})");
    assert_eq!(code, Some(0), "exit code (stderr: {stderr})");
}
