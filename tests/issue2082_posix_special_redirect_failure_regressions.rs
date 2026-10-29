//! rubash#2082 — a redirection failure on a POSIX special builtin
//! must exit a non-interactive POSIX-mode shell; rubash reported the
//! error and kept executing.
//!
//! GNU: execute_builtin_or_function's do_redirections failure returns
//! EX_REDIRFAIL (execute_cmd.c:5613), and execute_simple_command's
//! conversion (execute_cmd.c:4865-4875) makes a posixly_correct,
//! non-interactive shell exit with EXECUTION_FAILURE when the
//! builtin is special. The jump fires inside execute_simple_command,
//! so tested contexts (`||`, if conditions, `!`) do NOT contain it
//! and the ERR trap never runs (the caller-side check at
//! execute_cmd.c:994 is never reached); the EXIT trap still runs.
//! Non-special builtins, disk commands and functions report status 1
//! and continue (builtin_status conversion, execute_cmd.c:4896-4901),
//! and the `command` builtin's re-dispatch never carries
//! builtin_is_special, so `command : <&9` also continues.
//!
//! Rubash's failure surfaced as an IoError out of the builtin
//! redirect application, which execute_command's arm rendered and
//! then continued (only `set -e` converted it, with an ERR trap GNU
//! does not fire here). The fix adds the EX_REDIRFAIL conversion to
//! that arm, gated on the same class as GNU: posix mode,
//! non-interactive, first word in the 16-name special set, not under
//! a `command` re-dispatch, not `enable`-disabled to the filesystem.
//!
//! All expectations measured against GNU bash 5.3.20 with script
//! files (self-compiled oracle, 2026-10-29). Every `<&9` script
//! starts with `exec 9>&-` so fd 9 is unbound no matter what the
//! test harness inherited.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2082-{tag}-{}", std::process::id()));
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

/// One posix `<&9` case per special name: the shell must exit 1 with
/// the redirection diagnostic and never reach the AFTER line.
fn assert_posix_special_exits(tag: &str, command_line: &str) {
    let script = format!("exec 9>&-\nset -o posix\n{command_line}\necho \"AFTER\"\n");
    let (stdout, stderr, code) = run(tag, &script);
    assert_eq!(stdout, "", "{tag}: stdout: {stdout} stderr: {stderr}");
    assert!(
        stderr.contains("9: Bad file descriptor"),
        "{tag}: stderr: {stderr}"
    );
    assert_eq!(code, Some(1), "{tag}: exit code");
}

#[test]
fn issue2082_posix_dot_redirect_failure_exits() {
    assert_posix_special_exits("dot", ". /dev/null <&9");
}

#[test]
fn issue2082_posix_source_redirect_failure_exits() {
    assert_posix_special_exits("source", "source /dev/null <&9");
}

#[test]
fn issue2082_posix_colon_redirect_failure_exits() {
    assert_posix_special_exits("colon", ": <&9");
}

#[test]
fn issue2082_posix_eval_redirect_failure_exits() {
    assert_posix_special_exits("eval", "eval true <&9");
}

#[test]
fn issue2082_posix_export_redirect_failure_exits() {
    // A second special family (assignment builtin): same class.
    assert_posix_special_exits("export", "export <&9");
}

#[test]
fn issue2082_posix_colon_missing_file_redirect_exits() {
    // The failure class is do_redirections itself, not just bad dups:
    // a failing file open on a special builtin exits too.
    let (stdout, stderr, code) = run(
        "missing-file",
        "set -o posix\n: < /nonexistent-rubash2082\necho \"AFTER\"\n",
    );
    assert_eq!(stdout, "", "stdout: {stdout} stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn issue2082_posix_or_context_does_not_contain_the_exit() {
    // GNU's jump fires inside execute_simple_command: even as the
    // left operand of || the shell exits — no CAUGHT, no AFTER.
    let (stdout, stderr, code) = run(
        "or-context",
        "exec 9>&-\nset -o posix\n: <&9 || echo CAUGHT\necho \"AFTER\"\n",
    );
    assert_eq!(stdout, "", "stdout: {stdout} stderr: {stderr}");
    assert!(
        stderr.contains("9: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn issue2082_posix_exit_trap_runs_but_err_trap_does_not() {
    let (stdout, stderr, code) = run(
        "traps",
        "exec 9>&-\nset -o posix\ntrap 'echo ERRTRAP' ERR\ntrap 'echo EXITTRAP' EXIT\n: <&9\necho \"AFTER\"\n",
    );
    assert_eq!(stdout, "EXITTRAP\n", "stdout: {stdout} stderr: {stderr}");
    assert!(
        stderr.contains("9: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn issue2082_posix_subshell_contains_the_exit() {
    // Inside a subshell the jump ends only the subshell (status 1);
    // the parent continues.
    let (stdout, stderr, code) = run(
        "subshell",
        "exec 9>&-\nset -o posix\n( : <&9 )\necho \"AFTER rc=$?\"\n",
    );
    assert_eq!(stdout, "AFTER rc=1\n", "stdout: {stdout} stderr: {stderr}");
    assert!(
        stderr.contains("9: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2082_posix_colon_output_open_failure_exits() {
    // The apply-layer channel: an OUTPUT open failure passes the
    // reject gate (no dup/nounset fault) and fails inside the
    // builtin's redirect application — the same EX_REDIRFAIL class
    // through the IoError arm's conversion.
    let (stdout, stderr, code) = run(
        "output-open",
        "set -o posix\n: > /nonexistent-dir-rubash2082/f\necho \"AFTER\"\n",
    );
    assert_eq!(stdout, "", "stdout: {stdout} stderr: {stderr}");
    assert!(
        stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn issue2082_posix_pipeline_stage_redirect_failure_is_contained() {
    // A special builtin as a pipeline element dies as the element
    // (stage status 1); the pipeline completes and the parent shell
    // continues — the jump belongs to the element's subshell.
    let (stdout, stderr, code) = run(
        "stage-contained",
        "exec 9>&-\nset -o posix\n: <&9 | cat\necho \"AFTER prc=${PIPESTATUS[0]}\"\n",
    );
    assert_eq!(
        stdout, "AFTER prc=1\n",
        "stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stderr.contains("9: Bad file descriptor"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2082_default_mode_special_redirect_failure_continues() {
    // Control: no posix mode — report and continue, per-command
    // status 1, script status 0.
    let (stdout, stderr, code) = run(
        "default-mode",
        "exec 9>&-\n: <&9\necho \"AFTER-colon rc=$?\"\neval true <&9\necho \"AFTER-eval rc=$?\"\n",
    );
    assert_eq!(
        stdout, "AFTER-colon rc=1\nAFTER-eval rc=1\n",
        "stdout: {stdout} stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2082_posix_non_special_and_external_continue() {
    // Control: an ordinary builtin and a disk command with the same
    // failing dup report status 1 and the script continues.
    let (stdout, stderr, code) = run(
        "non-special",
        "exec 9>&-\nset -o posix\necho hi <&9\necho \"AFTER-echo rc=$?\"\ncat <&9\necho \"AFTER-cat rc=$?\"\n",
    );
    assert_eq!(
        stdout, "AFTER-echo rc=1\nAFTER-cat rc=1\n",
        "stdout: {stdout} stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2082_posix_function_redirect_failure_continues() {
    // Control: a function's own redirection failure is the function
    // branch of GNU's conversion (builtin_status -> 1), never an exit.
    let (stdout, stderr, code) = run(
        "function",
        "exec 9>&-\nset -o posix\nf() { echo INFUNC; }\nf <&9\necho \"AFTER rc=$?\"\n",
    );
    assert_eq!(
        stdout, "AFTER rc=1\n",
        "stdout: {stdout} stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue2082_posix_command_prefix_redirect_failure_continues() {
    // Control: under the `command` builtin's re-dispatch the inner
    // command does not carry builtin_is_special (GNU errors8.sub
    // family) — report and continue.
    let (stdout, stderr, code) = run(
        "command-prefix",
        "exec 9>&-\nset -o posix\ncommand : <&9\necho \"AFTER rc=$?\"\n",
    );
    assert_eq!(
        stdout, "AFTER rc=1\n",
        "stdout: {stdout} stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
