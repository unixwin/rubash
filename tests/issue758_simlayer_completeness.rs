//! Issue niubash#758 (Windows POSIX simulation-layer completeness):
//! the faces the #1898/#1900/#1917/#1935 waves left missing — the
//! entropy devices (/dev/random, /dev/urandom) as a registry class,
//! and the procps per-pid faces (/proc/<pid>/stat, statm, environ),
//! /proc/mounts, and /proc/net/dev.
//!
//! GNU anchors (coreutils/procps consumers on real device/procfs
//! nodes): an entropy device reads as an unbounded non-repeating
//! byte stream and accepts writes that are discarded; test(1)
//! reports it exactly like /dev/null (crw-rw-rw-: -e -r -w -c true,
//! -f -x -s false). /proc/<pid>/stat is the 52-field line, statm is
//! seven page counts, environ is the NUL-separated block, mounts
//! lines carry six fields, and /proc/net/dev rows carry sixteen
//! counters under the two header lines.
//!
//! Windows-gated: off Windows the real devices and procfs serve
//! these opens, and the whole file compiles empty.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash758-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn run(script: &str) -> Run {
    let dir = tempdir("case");
    rubash_c(script, &dir)
}

#[test]
fn issue758_entropy_reads_are_unbounded_real_bytes() {
    // External consumer through the input ladder: exactly the
    // requested byte count arrives (the pump-fed CSPRNG source).
    let out = run("head -c 64 /dev/urandom | wc -c");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "64", "stdout: {}", out.stdout);
    let out = run("head -c 16 < /dev/random | wc -c");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "16", "stdout: {}", out.stdout);
    // The stream is not a constant fill: two 32-byte draws differ
    // (a collision is a 2^-256 event, not a test risk).
    let out = run("a=$(head -c 32 /dev/urandom | od -An -tx1); \
         b=$(head -c 32 /dev/urandom | od -An -tx1); \
         if [ \"$a\" != \"$b\" ]; then echo differ; else echo same; fi");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "differ", "stdout: {}", out.stdout);
}

#[test]
fn issue758_entropy_bound_fds_and_readwrite() {
    // A bound descriptor consumes real bytes (status 0), never a
    // spurious EOF (status 1).
    let out = run("exec 3</dev/urandom; read -n 4 -u 3 z; echo rc=$?");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "rc=0", "stdout: {}", out.stdout);
    // `<> /dev/urandom` binds read+write; both sides serve.
    let out = run("exec 3<>/dev/urandom; read -n 2 -u 3 z; echo rc=$?; echo x >&3; echo wrc=$?");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "rc=0\nwrc=0", "stdout: {}", out.stdout);
}

#[test]
fn issue758_entropy_writes_discard_and_succeed() {
    for script in [
        "echo ok > /dev/urandom; echo rc=$?",
        "echo ok > /dev/random; echo rc=$?",
        "echo ok >> /dev/urandom; echo rc=$?",
    ] {
        let out = run(script);
        assert_eq!(out.code, Some(0), "script {script} stderr: {}", out.stderr);
        assert_eq!(out.stdout.trim(), "rc=0", "script {script}");
        assert_eq!(out.stderr, "", "script {script}");
    }
    // A bound write descriptor discards the same way.
    let out = run("exec 7>/dev/urandom; echo x >&7; echo rc=$?");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "rc=0", "stdout: {}", out.stdout);
}

#[test]
fn issue758_entropy_test_builtin_table() {
    // The /dev/null truth table (crw-rw-rw-), per the GNU oracle.
    let out = run(
        "[ -e /dev/urandom ] && [ -c /dev/urandom ] && [ -r /dev/urandom ] \
         && [ -w /dev/urandom ] && [ -e /dev/random ] && echo yes",
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "yes", "stdout: {}", out.stdout);
    let out = run("[ -f /dev/urandom ] || [ -x /dev/urandom ] || echo neither");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "neither", "stdout: {}", out.stdout);
}

#[test]
fn issue758_proc_pid_stat_statm_environ_shapes() {
    // 52 fields, the pid itself in field 1.
    let out = run("set -- $(cat /proc/self/stat); echo $#");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "52", "stdout: {}", out.stdout);
    let out = run("set -- $(cat /proc/self/statm); echo $#");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "7", "stdout: {}", out.stdout);
    // environ is the true block: the harness environment's PATH
    // entry survives the NUL separation.
    let out = run("tr '\\0' '\\n' < /proc/self/environ | grep -c '^PATH='");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "1", "stdout: {}", out.stdout);
    // status carries the memory face lines.
    let out = run("grep -c '^VmRSS:' /proc/self/status");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "1", "stdout: {}", out.stdout);
    // The numeric spelling serves the same faces.
    let out = run("set -- $(cat /proc/$$/stat); echo $#");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "52", "stdout: {}", out.stdout);
}

#[test]
fn issue758_proc_mounts_and_net_dev_shapes() {
    // Six fields per mounts line (first line is a volume line).
    let out = run("set -- $(head -1 /proc/mounts); echo $#");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "6", "stdout: {}", out.stdout);
    // net/dev: two header lines plus at least the loopback row.
    let out = run("wc -l < /proc/net/dev");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let lines: i64 = out.stdout.trim().parse().expect("line count");
    assert!(lines >= 3, "net/dev lines: {lines}");
    // The fd directory exists as a directory.
    let out = run("[ -d /proc/self/fd ] && echo dir");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "dir", "stdout: {}", out.stdout);
}
