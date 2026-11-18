//! niubash#993 S4 — an oversize command line must report the E2BIG
//! strerror, not the Win32 name for the failing syscall.
//!
//! GNU anchor (execute_cmd.c shell_execve): a failed execve whose
//! errno is E2BIG/ENOMEM short-circuits the interpreter-sample
//! classification (`:6156` `file_error (command)` -> `sys_error` ->
//! `strerror(E2BIG)` = "Argument list too long"), status EX_NOEXEC
//! (126, `:6143`). On Windows the same condition surfaces as
//! ERROR_FILENAME_EXCED_RANGE (206) from CreateProcess: a generic
//! strerror table renders 206 "File name too long", which is right
//! for path opens but wrong for the spawn classification.
//!
//! Repro from the issue:
//!   arg=$(printf 'a%.0s' $(seq 1 40000)); python -c '...' "$arg"
//! -> Git Bash `python.exe: Argument list too long`; rubash printed
//! `python.exe: File name too long`.

#![cfg(windows)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_script(tag: &str, script: &str) -> (String, String, i32) {
    let dir = std::env::temp_dir().join(format!("rubash-993s4-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path: PathBuf = dir.join("case.sh");
    std::fs::write(&path, script).expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&path)
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "SystemRoot",
        "SystemDrive",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "OS",
        "TEMP",
        "TMP",
    ] {
        if let Ok(value) = std::env::var(name) {
            cmd.env(name, value);
        }
    }
    let out = cmd.output().expect("run rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// A 200 KiB single argument is past every limit: CreateProcess's
/// 32767-char lpCommandLine on Windows (and execve's MAX_ARG_STRLEN
/// on unix) — E2BIG must report the GNU wording.
#[test]
fn oversize_command_line_reports_argument_list_too_long() {
    let (_stdout, stderr, _code) = run_script(
        "e2big",
        "arg=$(printf 'a%.0s' {1..200000})\n\"$COMSPEC\" x \"$arg\"\necho \"rc=$?\"\n",
    );
    assert!(
        stderr.contains("Argument list too long"),
        "stderr: {stderr:?}"
    );
    assert!(!stderr.contains("File name too long"), "stderr: {stderr:?}");
}

#[test]
fn oversize_command_line_exits_126() {
    let (stdout, _stderr, _code) = run_script(
        "rc",
        "arg=$(printf 'a%.0s' {1..200000})\n\"$COMSPEC\" x \"$arg\"\necho \"rc=$?\"\n",
    );
    assert_eq!(stdout.trim(), "rc=126", "stdout: {stdout:?}");
}

/// The exec builtin runs the same classification (exec.def ->
/// shell_execve): wording identical, with the exec-specific prefix.
#[test]
fn exec_builtin_reports_argument_list_too_long() {
    let (_stdout, stderr, _code) = run_script(
        "exec",
        "arg=$(printf 'a%.0s' {1..200000})\nexec \"$COMSPEC\" x \"$arg\"\n",
    );
    assert!(
        stderr.contains("Argument list too long"),
        "stderr: {stderr:?}"
    );
}
