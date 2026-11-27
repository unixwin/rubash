//! rubash#2127 — two control-flow builtin error-report faces:
//!
//! 1. `break abc` / `continue abc` inside a loop, POSIX mode: GNU's
//!    get_numeric_arg failure is jump_to_top_level (EXITPROG) in a
//!    non-interactive shell (builtins/break.def:76 get_numeric_arg,
//!    builtins/common.c:488-510) — the shell dies with status 2 and
//!    `||` does not contain it. rubash's unwind already matched, but
//!    `execute_loop_control`'s NotNumeric arm returned
//!    Err(ExitCode(2)) without parking `self.exit_code = 2`; the POSIX
//!    completion channel reads the parked code, so a POSIX script (or
//!    POSIX `-c`) exited with the stale code (0). The default-mode
//!    error channel carries the code itself and already yielded 2 —
//!    those shapes are pinned below as guards.
//! 2. `return abc` at the top level in POSIX mode: GNU return.def:57-69
//!    returns EX_USAGE raw (posix + non-interactive + not under
//!    `command`) BEFORE the return_catch check, so only the
//!    "numeric argument required" diagnostic prints — the
//!    "can only `return' ..." line never appears. rubash printed both.
//!    Default mode keeps BOTH lines (also pinned below as a guard).
//!
//! All expectations measured against GNU bash 5.3.20 (self-compiled
//! oracle) with script files and `-c` strings, 2026-10-30.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2127-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
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

fn run_c(tag: &str, command: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .args(["-c", command])
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

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

#[test]
fn break_abc_in_loop_script_exits_2_issue2127() {
    let (stdout, stderr, code) =
        run_script("breakabc", "while :; do\n  break abc\ndone\necho AFTER\n");
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_eq!(
        count_occurrences(&stderr, "break: abc: numeric argument required"),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn continue_abc_in_loop_script_exits_2_issue2127() {
    let (stdout, stderr, code) = run_script(
        "contabc",
        "i=0\nwhile [ $i -lt 2 ]; do\n  i=$((i+1))\n  continue abc\ndone\necho AFTER\n",
    );
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_eq!(
        count_occurrences(&stderr, "continue: abc: numeric argument required"),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn break_abc_or_shape_script_exits_2_issue2127() {
    // GNU: the EXITPROG jump is not contained by `||` — no RESCUED, no
    // AFTER, the script dies with status 2.
    let (stdout, stderr, code) = run_script(
        "breakor",
        "while :; do\n  break abc || echo RESCUED\ndone\necho AFTER\n",
    );
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_eq!(
        count_occurrences(&stderr, "break: abc: numeric argument required"),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn break_abc_c_string_exits_2_issue2127() {
    // Guard: the -c path already yielded 2 before the fix.
    let (stdout, stderr, code) = run_c("breakc", "while :; do break abc; done");
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(
        count_occurrences(&stderr, "break: abc: numeric argument required"),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn break_abc_posix_script_exits_2_issue2127() {
    // The POSIX completion channel reads the parked exit_code: the
    // diagnostic prints and the unwind runs, but without the park the
    // shell exited with the stale code (0).
    let (stdout, stderr, code) = run_script(
        "breakposix",
        "set -o posix\nwhile :; do\n  break abc\ndone\necho AFTER\n",
    );
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_eq!(
        count_occurrences(&stderr, "break: abc: numeric argument required"),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn continue_abc_posix_script_exits_2_issue2127() {
    let (stdout, stderr, code) = run_script(
        "contposix",
        "set -o posix\ni=0\nwhile [ $i -lt 2 ]; do\n  i=$((i+1))\n  continue abc\ndone\necho AFTER\n",
    );
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_eq!(
        count_occurrences(&stderr, "continue: abc: numeric argument required"),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn break_abc_posix_or_shape_and_c_string_exit_2_issue2127() {
    let (stdout, stderr, code) = run_script(
        "breakposixor",
        "set -o posix\nwhile :; do\n  break abc || echo RESCUED\ndone\necho AFTER\n",
    );
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    let (stdout, stderr, code) = run_c(
        "breakposixc",
        "set -o posix; while :; do break abc; done; echo AFTER",
    );
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_eq!(
        count_occurrences(&stderr, "break: abc: numeric argument required"),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn return_abc_posix_top_level_single_diagnostic_issue2127() {
    // GNU return.def:62-65: posix + non-interactive returns EX_USAGE
    // before the can-only-`return' report — exactly one diagnostic,
    // and the POSIX special-builtin channel exits the shell with 2.
    let (stdout, stderr, code) = run_script("retposix", "set -o posix\nreturn abc\necho AFTER\n");
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "", "stdout: {stdout}");
    assert_eq!(
        count_occurrences(&stderr, "return: abc: numeric argument required"),
        1,
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("can only `return'"),
        "stderr must not carry the second diagnostic: {stderr}"
    );
}

#[test]
fn return_abc_default_top_level_keeps_both_diagnostics_issue2127() {
    // Guard: in default mode GNU prints BOTH diagnostics (get_exitstat's
    // numeric report, then return_builtin's can-only report), the
    // builtin's status is 2, and the script runs on.
    let (stdout, stderr, code) = run_script("retdefault", "return abc\necho \"rc=$?\"\n");
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "rc=2\n", "stdout: {stdout}");
    assert_eq!(
        count_occurrences(&stderr, "return: abc: numeric argument required"),
        1,
        "stderr: {stderr}"
    );
    assert_eq!(
        count_occurrences(
            &stderr,
            "can only `return' from a function or sourced script"
        ),
        1,
        "stderr: {stderr}"
    );
}
