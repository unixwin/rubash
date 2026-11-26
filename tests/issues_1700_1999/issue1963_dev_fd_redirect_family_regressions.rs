//! rubash#1963: the /dev/fd/N redirect family on a CLOSED fd must
//! produce exactly GNU's single failure shape on every face.
//!
//! GNU 5.3.20 (measured, probes/p1_fd63.sh): redir.c performs one
//! open; a closed alias fails it ENOENT, and every kind — `>`, `<`,
//! `>>`, `<>`, on no-output commands, output builtins and externals
//! alike — reports exactly one line
//! `{script}: line N: /dev/fd/63: No such file or directory` with
//! status 1, and an input-side failure aborts the command (its
//! output never happens). The `Bad file descriptor` text belongs
//! only to true dup kinds (`<&N`/`>&N`). With fd 63 open, the same
//! shapes all succeed.
//!
//! Pre-fix rubash diverged per face: `: >` printed TWO lines (the
//! ENOENT line plus a `63: Bad file descriptor` line from the
//! no-output mirror treating the alias spelling as a dup source),
//! `: <>` reported the EBADF shape, `: >>` silently succeeded, and
//! `echo hi <` / `echo hi <>` silently succeeded AND printed hi.
//!
//! The pins assert the invariant — status 1, exactly one stderr
//! line, ending with the ENOENT text — rather than the diagnostic
//! prefix, which legitimately varies with invocation mode.

use std::process::Command;

fn rubash() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
}

fn run(script: &str) -> (String, String, i32) {
    let out = rubash()
        .arg("-c")
        .arg(script)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// The unified closed-alias failure: status 1, no stdout payload,
/// exactly one stderr line ending with the ENOENT text.
fn assert_single_enoent(script: &str) {
    let (stdout, stderr, code) = run(script);
    assert_eq!(
        code, 1,
        "script: {script} stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        !stdout.contains("hi"),
        "script: {script} — the command must be aborted, stdout: {stdout}"
    );
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "script: {script} — exactly one diagnostic line, stderr: {stderr}"
    );
    assert!(
        lines[0].ends_with("/dev/fd/63: No such file or directory")
            || lines[0].ends_with("/proc/self/fd/63: No such file or directory"),
        "script: {script} — line must carry the ENOENT text, got: {}",
        lines[0]
    );
}

#[test]
fn issue1963_closed_alias_output_kinds_fail_single_enoent() {
    assert_single_enoent(": > /dev/fd/63");
    assert_single_enoent("true > /dev/fd/63");
    assert_single_enoent("echo hi > /dev/fd/63");
    assert_single_enoent(": >> /dev/fd/63");
    assert_single_enoent("echo hi >> /dev/fd/63");
    assert_single_enoent(": 2> /dev/fd/63");
}

#[test]
fn issue1963_closed_alias_input_kinds_abort_single_enoent() {
    assert_single_enoent(": < /dev/fd/63");
    assert_single_enoent("echo hi < /dev/fd/63");
    assert_single_enoent(": <> /dev/fd/63");
    assert_single_enoent("echo hi <> /dev/fd/63");
    assert_single_enoent("cat < /dev/fd/63");
}

#[test]
fn issue1963_closed_alias_read_reports_enoent_once() {
    let (stdout, stderr, code) = run("read x < /dev/fd/63; echo \"rc=$? x=[$x]\"");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "rc=1 x=[]", "stdout: {stdout}");
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "stderr: {stderr}");
    assert!(
        lines[0].ends_with("/dev/fd/63: No such file or directory"),
        "line: {}",
        lines[0]
    );
}

#[test]
fn issue1963_closed_alias_proc_spelling_parity() {
    // The registry classifies /proc/self/fd/N as the same alias;
    // the failure shape must not depend on the spelling.
    assert_single_enoent(": > /proc/self/fd/63");
    assert_single_enoent(": < /proc/self/fd/63");
}

#[test]
fn issue1963_closed_alias_exec_bind_fails_single_enoent() {
    // `exec 3> /dev/fd/63` performs the same open at bind time:
    // one ENOENT line, the exec reports status 1, and the script
    // continues (measured on the oracle).
    let (stdout, stderr, code) = run("exec 3> /dev/fd/63; echo rc=$?");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "rc=1", "stdout: {stdout}");
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "stderr: {stderr}");
    assert!(
        lines[0].ends_with("/dev/fd/63: No such file or directory"),
        "line: {}",
        lines[0]
    );
}

#[test]
fn issue1963_open_alias_shapes_all_succeed() {
    let script = "exec 63>/dev/null; \
                  : > /dev/fd/63; echo a=$?; \
                  : < /dev/fd/63; echo b=$?; \
                  : >> /dev/fd/63; echo c=$?; \
                  : <> /dev/fd/63; echo d=$?; \
                  exec 63>&-";
    let (stdout, stderr, code) = run(script);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "a=0\nb=0\nc=0\nd=0", "stdout: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
}
