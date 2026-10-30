//! rubash#2128 — the #1229 family's residual `--help` faces:
//!
//! 1. `break --help` / `continue --help` had no CHECK_HELPOPT face in
//!    loop control at all. GNU builtins/common.h:29-36 CHECK_HELPOPT
//!    inspects the RAW first word before check_loop_level /
//!    get_numeric_arg: the long help goes to stdout and the builtin
//!    returns EX_USAGE (2) WITHOUT unwinding — inside a loop body the
//!    body runs on; outside a loop there is no "only meaningful"
//!    diagnostic. In POSIX non-interactive mode the EX_USAGE class
//!    (> EX_SHERRBASE) exits the shell (POSIX-fatal).
//! 2. `set --help` printed only the 2-line invalid-option usage. GNU
//!    set.def:671-691: the internal_getopt validity pre-scan returns
//!    GETOPT_HELP for a `--help` word in option position
//!    (builtins/bashgetopt.c:80-85) and CASE_HELPOPT (set.def:684)
//!    prints the full long help and returns EX_USAGE — with NOTHING
//!    applied, because the scan runs before the apply loop
//!    (`set -e --help` leaves errexit off). Words after `--` or after
//!    a positional are not option position and never trigger it.
//!
//! The long texts themselves already live in help_texts.rs (the `help`
//! builtin's pages, byte-verified against GNU); `set --help` is pinned
//! byte-equal to `help set`.
//!
//! All expectations measured against GNU bash 5.3.20 (self-compiled
//! oracle), 2026-10-30.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2128-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
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

const BREAK_HELP: &str = "break: break [n]\n    Exit for, while, or until loops.\n    \n    Exit a FOR, WHILE or UNTIL loop.  If N is specified, break N enclosing\n    loops.\n    \n    Exit Status:\n    The exit status is 0 unless N is not greater than or equal to 1.\n";

const CONTINUE_HELP: &str = "continue: continue [n]\n    Resume for, while, or until loops.\n    \n    Resumes the next iteration of the enclosing FOR, WHILE or UNTIL loop.\n    If N is specified, resumes the Nth enclosing loop.\n    \n    Exit Status:\n    The exit status is 0 unless N is not greater than or equal to 1.\n";

#[test]
fn break_help_prints_full_gnu_text_issue2128() {
    let (stdout, stderr, code) = run_c("breakhelp", "break --help");
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, BREAK_HELP, "stdout: {stdout:?}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn continue_help_prints_full_gnu_text_issue2128() {
    let (stdout, stderr, code) = run_c("conthelp", "continue --help");
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, CONTINUE_HELP, "stdout: {stdout:?}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn break_help_inside_loop_body_continues_issue2128() {
    // CHECK_HELPOPT returns EX_USAGE without setting `breaking`: the
    // loop body runs on after the help, with $? = 2.
    let (stdout, stderr, code) = run_script(
        "breakhelpbody",
        "i=0\nwhile [ $i -lt 1 ]; do\n  i=1\n  break --help >/dev/null\n  echo \"rc=$?\"\n  break\ndone\n",
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "rc=2\n", "stdout: {stdout}");
}

#[test]
fn break_help_posix_script_exits_issue2128() {
    // POSIX-fatal: the EX_USAGE class exits a non-interactive POSIX
    // shell even outside a loop; AFTER never prints.
    let (stdout, stderr, code) =
        run_script("breakhelpposix", "set -o posix\nbreak --help\necho AFTER\n");
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, BREAK_HELP, "stdout: {stdout:?}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn set_help_prints_full_gnu_text_issue2128() {
    let (stdout, stderr, code) = run_c("sethelp", "set --help");
    let (help_stdout, help_stderr, help_code) = run_c("sethelppage", "help set");
    assert_eq!(help_code, Some(0), "help set: {help_stderr}");
    assert_eq!(code, Some(2), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert!(
        stdout.starts_with("set: set [-abefhkmnptuvxBCEHPT] [-o option-name]"),
        "first line: {:?}",
        stdout.lines().next()
    );
    assert_eq!(stdout, help_stdout, "set --help must equal the help page");
}

#[test]
fn set_help_applies_nothing_issue2128() {
    // The getopt pre-scan aborts at --help before the apply loop:
    // `set -e --help` must NOT turn errexit on, so `false` is survived.
    let (stdout, stderr, code) = run_script(
        "sethelpnoapply",
        "set -e --help >/dev/null\nfalse\necho SURVIVED\n",
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "SURVIVED\n", "stdout: {stdout}");
}

#[test]
fn set_help_after_double_dash_is_positional_issue2128() {
    // `--` ends option position: --help becomes $1, no help is printed.
    let (stdout, stderr, code) = run_c("sethelpdd", "set -- --help; echo \"p1=$1\"");
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout, "p1=--help\n", "stdout: {stdout}");
}
