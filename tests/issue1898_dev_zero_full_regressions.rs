//! rubash#1898: /dev/zero and /dev/full join the special-path
//! registry as device classes. GNU (measured against the owner-built
//! 5.3.20 oracle): reading either device yields an unbounded stream
//! of zero bytes; `> /dev/zero` discards and succeeds; a write to
//! /dev/full fails at write time with
//! `echo: write error: No space left on device` (sh_chkwrite,
//! builtins/common.c:320-334) and status 1, appends identically; the
//! test table for both devices equals /dev/null's (exists, readable,
//! writable, character device; not block/file/size).
//!
//! These pins run the redirect/service faces only, with bounded
//! shapes (an unbounded /dev/zero read would hang by design). The
//! faces are platform-neutral code (the classes answer identically
//! on every host), so the pins are ungated.

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

#[test]
fn issue1898_zero_reads_are_bounded_zero_bytes() {
    // `od -An -tu1` pads each field to its display width, so the
    // byte list is compared whitespace-normalized.
    let zeros = |script: &str| {
        let (stdout, stderr, code) = run(script);
        assert_eq!(code, 0, "stderr: {stderr}");
        stdout.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    // External consumer through the input ladder: the pump-fed
    // source must deliver exactly the requested zeros.
    assert_eq!(zeros("head -c 4 < /dev/zero | od -An -tu1"), "0 0 0 0");
    // /dev/full reads as zeros too (its write side is the full one).
    assert_eq!(zeros("head -c 3 < /dev/full | od -An -tu1"), "0 0 0");
}

#[test]
fn issue1898_bound_fd_reads_zeros_not_eof() {
    // `exec 3</dev/zero` binds the source; read -n must consume real
    // bytes (status 0), not mistake the stream for EOF (status 1).
    let (stdout, stderr, code) = run("exec 3</dev/zero; read -n 4 -u 3 z; echo rc=$?");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "rc=0", "stdout: {stdout} stderr: {stderr}");

    // `<> /dev/zero` binds read+write; the read side serves zeros.
    let (stdout, stderr, code) = run("exec 3<>/dev/zero; read -n 2 -u 3 z; echo rc=$?");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "rc=0", "stdout: {stdout} stderr: {stderr}");
}

#[test]
fn issue1898_zero_writes_discard_and_succeed() {
    let (stdout, stderr, code) = run("echo ok > /dev/zero; echo rc=$?");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "rc=0");
    assert_eq!(stderr, "");

    let (_stdout, stderr, code) = run("echo ok >> /dev/zero");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stderr, "");
}

#[test]
fn issue1898_full_writes_fail_enospc_in_the_builtins_name() {
    for script in [
        "echo hi > /dev/full",
        "echo hi >> /dev/full",
        "printf 'x' > /dev/full",
    ] {
        let (stdout, stderr, code) = run(script);
        let name = if script.starts_with("printf") {
            "printf"
        } else {
            "echo"
        };
        assert_eq!(code, 1, "script: {script} stderr: {stderr}");
        assert_eq!(stdout, "", "script: {script}");
        assert!(
            stderr.contains(&format!("{name}: write error: No space left on device")),
            "script: {script} stderr: {stderr}"
        );
    }
}

#[test]
fn issue1898_bound_full_fd_write_fails_enospc() {
    // `exec 7>/dev/full` binds; a later write through the binding
    // fails with the same sh_chkwrite shape.
    let (stdout, stderr, code) = run("exec 7>/dev/full; echo x >&7; echo rc=$?");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "rc=1", "stdout: {stdout} stderr: {stderr}");
    assert!(
        stderr.contains("echo: write error: No space left on device"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1898_test_table_matches_dev_null() {
    // GNU: for /dev/zero, /dev/full and /dev/null alike —
    // -e -r -w -c true; -x -b -f -s -d false.
    let script = r#"
for dev in /dev/zero /dev/full /dev/null; do
  line="$dev:"
  for op in e r w x c b f s d; do
    if test -$op $dev; then line="$line $op=1"; else line="$line $op=0"; fi
  done
  echo "$line"
done"#;
    let (stdout, stderr, code) = run(script);
    assert_eq!(code, 0, "stderr: {stderr}");
    for dev in ["/dev/zero", "/dev/full", "/dev/null"] {
        let expected = format!("{dev}: e=1 r=1 w=1 x=0 c=1 b=0 f=0 s=0 d=0");
        assert!(
            stdout.lines().any(|line| line == expected),
            "missing row {expected:?} in stdout: {stdout}"
        );
    }
}
