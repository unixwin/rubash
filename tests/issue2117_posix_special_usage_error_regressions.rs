//! rubash#2117 — a POSIX special builtin's USAGE error (the
//! > EX_SHERRBASE class: EX_USAGE, and the trap reset-by-name raw
//! EX_USAGE face) must exit a non-interactive POSIX-mode shell;
//! rubash reported the error and kept executing for the names whose
//! wrappers never marked `special_builtin_failed` (export, readonly,
//! trap, exec, eval, `.`/`source`).
//!
//! GNU: the builtin returns EX_USAGE (258 > EX_SHERRBASE, shell.h:71);
//! execute_simple_command converts it with builtin_status
//! (execute_cmd.c:4959-4979: EX_USAGE -> EX_BADUSAGE 2) and marks
//! special_builtin_failed (execute_cmd.c:4886-4888); the completion
//! check (execute_cmd.c:1004-1017) then exits the shell with that
//! status — gated on `ignore_return == 0 && invert == 0`, so tested
//! contexts (`||`, `&&`, if/while/until conditions, `!`) DO contain
//! this deferred channel (unlike #2082's EX_REDIRFAIL, which jumps
//! inside execute_simple_command at execute_cmd.c:4865-4875 and is
//! not contained). The ERR trap fires first (execute_cmd.c:993-1001)
//! and its own commands reset the flag at execute_cmd.c:4480, so a
//! benign ERR trap rescues the shell; the EXIT trap still runs on
//! the way out. Non-special builtins (cd) and plain failures of
//! special builtins (shift out-of-range, bad trap signal spec,
//! `exit 1 2`) are not this class and never exit.
//!
//! All expectations measured against GNU bash 5.3.20 with script
//! files (self-compiled oracle, 2026-10-29).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2117-{tag}-{}", std::process::id()));
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

/// A bare usage error of a special builtin in POSIX mode: the shell
/// exits 2 after the diagnostic and never reaches the AFTER line.
fn assert_posix_usage_exits(tag: &str, command_line: &str, diagnostic: &str) {
    let script = format!("set -o posix\n{command_line}\necho \"AFTER\"\n");
    let (stdout, stderr, code) = run(tag, &script);
    assert_eq!(stdout, "", "{tag}: stdout: {stdout} stderr: {stderr}");
    assert!(
        stderr.contains(diagnostic),
        "{tag}: stderr should contain {diagnostic:?}: {stderr}"
    );
    assert_eq!(code, Some(2), "{tag}: exit code (stderr: {stderr})");
}

/// The same command in POSIX mode must NOT exit the shell.
fn assert_posix_continues(tag: &str, script_body: &str) {
    let script = format!("set -o posix\n{script_body}\n");
    let (stdout, _stderr, code) = run(tag, &script);
    assert!(
        stdout.contains("AFTER"),
        "{tag}: stdout should reach AFTER: {stdout}"
    );
    assert_eq!(code, Some(0), "{tag}: exit code (stdout: {stdout})");
}

#[test]
fn issue2117_posix_export_invalid_option_exits() {
    assert_posix_usage_exits("export-opt", "export -z", "invalid option");
}

#[test]
fn issue2117_posix_readonly_invalid_option_exits() {
    assert_posix_usage_exits("readonly-opt", "readonly -z", "invalid option");
}

#[test]
fn issue2117_posix_export_functions_invalid_option_exits() {
    assert_posix_usage_exits("export-f-opt", "export -fz", "invalid option");
}

#[test]
fn issue2117_posix_readonly_functions_invalid_option_exits() {
    assert_posix_usage_exits("readonly-f-opt", "readonly -fz", "invalid option");
}

#[test]
fn issue2117_posix_trap_invalid_option_exits() {
    assert_posix_usage_exits("trap-opt", "trap -z", "invalid option");
}

#[test]
fn issue2117_posix_trap_print_actions_without_signal_exits() {
    assert_posix_usage_exits("trap-P", "trap -P", "requires at least one signal");
}

#[test]
fn issue2117_posix_trap_reset_by_name_exits() {
    // The raw-EX_USAGE face: uniform verdict through the one gate.
    assert_posix_usage_exits("trap-reset", "trap USR1", "usage: trap");
}

#[test]
fn issue2117_posix_exec_invalid_option_exits() {
    assert_posix_usage_exits("exec-opt", "exec -z", "invalid option");
}

#[test]
fn issue2117_posix_exec_help_exits() {
    let (stdout, _stderr, code) = run("exec-help", "set -o posix\nexec --help\necho \"AFTER\"\n");
    assert!(stdout.contains("exec"), "stdout: {stdout}");
    assert!(!stdout.contains("AFTER"), "stdout: {stdout}");
    assert_eq!(code, Some(2));
}

#[test]
fn issue2117_posix_eval_invalid_option_exits() {
    assert_posix_usage_exits("eval-opt", "eval -x", "invalid option");
}

#[test]
fn issue2117_posix_dot_no_filename_exits() {
    assert_posix_usage_exits("dot-noargs", ".", "filename argument required");
}

#[test]
fn issue2117_posix_dot_invalid_option_exits() {
    assert_posix_usage_exits("dot-opt", ". -z", "invalid option");
}

#[test]
fn issue2117_posix_set_invalid_option_exits() {
    // Already wired before #2117; the class sample pins the uniform rule.
    assert_posix_usage_exits("set-opt", "set -z", "invalid option");
}

#[test]
fn issue2117_posix_shift_non_numeric_exits() {
    assert_posix_usage_exits("shift-badarg", "shift abc", "numeric argument required");
}

#[test]
fn issue2117_posix_unset_invalid_option_exits() {
    assert_posix_usage_exits("unset-opt", "unset -z", "invalid option");
}

#[test]
fn issue2117_posix_exit_non_numeric_exits() {
    assert_posix_usage_exits("exit-badarg", "exit abc", "numeric argument required");
}

#[test]
fn issue2117_posix_usage_error_inside_function_exits() {
    let (stdout, _stderr, code) = run(
        "func-ctx",
        "set -o posix\nf() { export -z; echo \"FUNC_AFTER\"; }\nf\necho \"AFTER\"\n",
    );
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_eq!(code, Some(2));
}

#[test]
fn issue2117_posix_plain_failures_of_special_builtins_continue() {
    // Plain EXECUTION_FAILURE faces of special builtins are not the
    // > EX_SHERRBASE class: no exit (GNU builtin_status only converts
    // the internal codes; these return 1 directly).
    assert_posix_continues("shift-range", "shift 5\necho \"AFTER\"");
    assert_posix_continues("trap-badsig", "trap 'echo X' NOSUCHSIG\necho \"AFTER\"");
    assert_posix_continues("exit-toomany", "exit 1 2\necho \"AFTER\"");
    assert_posix_continues(
        "export-notfn",
        "export -f no_such_function_xyz\necho \"AFTER\"",
    );
}

#[test]
fn issue2117_posix_non_special_usage_error_continues() {
    assert_posix_continues("cd-opt", "cd -z\necho \"AFTER\"");
}

#[test]
fn issue2117_nonposix_usage_errors_continue() {
    for (tag, line) in [
        ("np-export", "export -z"),
        ("np-readonly", "readonly -z"),
        ("np-trap", "trap -z"),
        ("np-exec", "exec -z"),
        ("np-eval", "eval -x"),
        ("np-dot", "."),
    ] {
        let (stdout, _stderr, code) = run(tag, &format!("{line}\necho \"AFTER\"\n"));
        assert!(
            stdout.contains("AFTER"),
            "{tag}: stdout should reach AFTER: {stdout}"
        );
        assert_eq!(code, Some(0), "{tag}: exit code");
    }
}

#[test]
fn issue2117_posix_tested_contexts_contain_usage_errors() {
    assert_posix_continues("or", "export -z || echo \"OR_RAN\"\necho \"AFTER\"");
    assert_posix_continues("if", "if export -z; then echo \"THEN\"; fi\necho \"AFTER\"");
    assert_posix_continues("negate", "! export -z\necho \"AFTER\"");
    assert_posix_continues("trap-or", "trap USR1 || echo \"OR_RAN\"\necho \"AFTER\"");
    assert_posix_continues("comsub", "x=$(export -z 2>/dev/null)\necho \"AFTER\"");
    assert_posix_continues("pipeline", "export -z | cat\necho \"AFTER\"");
}

#[test]
fn issue2117_posix_command_builtin_suppresses_usage_fatality() {
    // execute_cmd.c:4657-4666: the `command` re-dispatch never
    // carries builtin_is_special.
    assert_posix_continues("cmd-export", "command export -z\necho \"AFTER\"");
    assert_posix_continues("cmd-trap", "command trap USR1\necho \"AFTER\"");
}

#[test]
fn issue2117_posix_err_trap_rescues_usage_fatality() {
    // The ERR trap fires (execute_cmd.c:993-1001); its commands reset
    // special_builtin_failed (execute_cmd.c:4480), so the :1004 check
    // does not exit — exactly once, no double fire at the list loop.
    let (stdout, _stderr, code) = run(
        "errtrap-rescue",
        "set -o posix\ntrap 'echo ERRTRAP_RAN' ERR\nexport -z\necho \"AFTER\"\n",
    );
    assert_eq!(stdout.matches("ERRTRAP_RAN").count(), 1, "stdout: {stdout}");
    assert!(stdout.contains("AFTER"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2117_posix_err_trap_rearmed_by_special_failure_exits() {
    // A trap body that itself ends in a failing special builtin
    // re-arms the flag: the exit still fires, with the ORIGINAL
    // command's converted status.
    let (stdout, _stderr, code) = run(
        "errtrap-rearm",
        "set -o posix\ntrap 'export -z' ERR\nset -z\necho \"AFTER\"\n",
    );
    assert!(!stdout.contains("AFTER"), "stdout: {stdout}");
    assert_eq!(code, Some(2));
}

#[test]
fn issue2117_posix_exit_trap_runs_on_usage_fatality() {
    let (stdout, _stderr, code) = run(
        "exittrap",
        "set -o posix\ntrap 'echo EXITTRAP_RAN' EXIT\nexport -z\necho \"AFTER\"\n",
    );
    assert!(stdout.contains("EXITTRAP_RAN"), "stdout: {stdout}");
    assert!(!stdout.contains("AFTER"), "stdout: {stdout}");
    assert_eq!(code, Some(2));
}

#[test]
fn issue2117_posix_time_usage_error_reports_then_exits() {
    // Edge (a): GNU prints the timing report (the exit is adjudicated
    // in execute_time_command's caller, after print_time_info) and
    // then the shell exits with the converted status.
    let (stdout, stderr, code) = run(
        "time-usage",
        "set -o posix\ntime export -z\necho \"AFTER\"\n",
    );
    assert!(!stdout.contains("AFTER"), "stdout: {stdout}");
    assert!(stderr.contains("real"), "stderr: {stderr}");
    assert_eq!(code, Some(2));
}

#[test]
fn issue2117_posix_time_redirect_failure_reports_then_exits() {
    // Edge (a), the #2082 channel under `time`: already correct
    // before #2117 — pinned here as the sibling guard.
    let (stdout, stderr, code) = run(
        "time-redirfail",
        "set -o posix\ntime export XV > /nonexistent-rubash2117/f\necho \"AFTER\"\n",
    );
    assert!(!stdout.contains("AFTER"), "stdout: {stdout}");
    assert!(stderr.contains("real"), "stderr: {stderr}");
    assert_eq!(code, Some(1));
}
