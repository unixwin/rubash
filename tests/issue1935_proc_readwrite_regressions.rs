//! Issue rubash#1935 (Windows synthetic /proc): `<>` on a synthetic
//! /proc entry had no service face — the ReadWrite probe and bind
//! raw-opened the mapped literal path and reported ENOENT (or
//! created a phantom enclave file through `>`).
//!
//! PREMISE CORRECTION (wrong-pin doctrine): the filed seed claimed
//! GNU succeeds on `exec 3<>/proc/version`. The GNU 5.3.20 oracle
//! (script-file probes on real procfs) says otherwise — the correct
//! anchor is the per-entry mode matrix:
//!   - 0444 class (version/stat/cpuinfo/meminfo/loadavg/uptime):
//!     write-direction opens fail at open with EACCES
//!     (`exec 3<>/proc/version` → "Permission denied").
//!   - 0644 class (per-pid status/cmdline): the open SUCCEEDS, reads
//!     serve the generated content, writes fail at write time with
//!     EINVAL (`echo: write error: Invalid argument`).
//! The fix routes every ReadWrite/output face through the registry's
//! write class (proc_vfs::proc_content_write_allowed): 0444 refuses
//! with the EACCES shape; 0644 binds — reads serve the snapshot,
//! writes are absorbed by a sink (the per-write EINVAL text is the
//! documented residual of this face: no Windows handle reproduces
//! it, and the absorbed shape keeps the open/read contract exact).
//!
//! Windows-gated: off Windows the real procfs serves these opens.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1935-{tag}-{}", std::process::id()));
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

#[test]
fn issue1935_readwrite_on_0444_class_fails_eacces() {
    let dir = tempdir("ro-rw");
    for target in ["/proc/version", "/proc/cpuinfo", "/proc/meminfo"] {
        let run = rubash_c(&format!(": <> {target}; echo should-not-print"), &dir);
        assert_eq!(run.code, Some(1), "{target}: stdout={}", run.stdout);
        assert!(
            run.stderr.contains(&format!("{target}: Permission denied")),
            "{target}: stderr={}",
            run.stderr
        );
        assert!(
            !run.stdout.contains("should-not-print"),
            "{target}: stdout={}",
            run.stdout
        );
        // Exactly one diagnostic line.
        assert_eq!(
            run.stderr.lines().count(),
            1,
            "{target}: stderr={}",
            run.stderr
        );
    }
}

#[test]
fn issue1935_output_on_0444_class_fails_eacces() {
    let dir = tempdir("ro-out");
    let run = rubash_c("echo hi > /proc/version; echo should-not-print", &dir);
    assert_eq!(run.code, Some(1), "stdout={}", run.stdout);
    assert!(
        run.stderr.contains("/proc/version: Permission denied"),
        "stderr={}",
        run.stderr
    );
    assert!(!run.stdout.contains("should-not-print"));
}

#[test]
fn issue1935_readwrite_on_0644_class_binds_and_reads() {
    let dir = tempdir("rw-bind");
    let run = rubash_c(
        "exec 3<>/proc/self/status; read -u 3 line; echo \"got:$line\"; exec 3<&-",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(
        run.stdout.contains("got:Name:"),
        "stdout={}",
        run.stdout
    );
}

#[test]
fn issue1935_output_on_0644_class_is_absorbed_not_phantom() {
    let dir = tempdir("rw-absorb");
    // The open succeeds; the generated snapshot remains the read
    // truth afterwards (GNU: the write fails, so nothing persists —
    // the observable end state agrees).
    let run = rubash_c(
        "echo payload > /proc/self/status; echo rc=$?; \
         read -r first < /proc/self/status; echo \"first:$first\"",
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(run.stdout.contains("rc=0"), "stdout={}", run.stdout);
    assert!(
        run.stdout.contains("first:Name:"),
        "stdout={}",
        run.stdout
    );
    // No phantom file landed in the session cwd.
    assert!(
        !dir.join("proc").exists(),
        "a phantom enclave proc tree appeared in the cwd"
    );
}

#[test]
fn issue1935_compound_readwrite_on_0444_fails_eacces() {
    let dir = tempdir("compound");
    let run = rubash_c("{ read -u 3 q; echo should-not-print; } 3<>/proc/stat", &dir);
    assert_eq!(run.code, Some(1), "stdout={}", run.stdout);
    assert!(
        run.stderr.contains("/proc/stat: Permission denied"),
        "stderr={}",
        run.stderr
    );
    assert!(!run.stdout.contains("should-not-print"));
}
