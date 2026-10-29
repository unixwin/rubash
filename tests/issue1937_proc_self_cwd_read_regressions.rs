//! Issue rubash#1937 (Windows synthetic /proc): `< /proc/self/cwd`
//! failed with the mapped literal path's shape instead of GNU's
//! read-time EISDIR. GNU 5.3.20 oracle (script-file probes on real
//! procfs): the directory OPEN succeeds on every face —
//!   read q < /proc/self/cwd     → `read: 0: read error: Is a directory`, rc 1
//!   exec 3</proc/self/cwd       → binds, rc 0
//!   read -u 3 (after the bind)  → `read: 3: read error: Is a directory`, rc 1
//! (read.def:911: a hard read(2) failure is reported, not treated as
//! EOF.) Windows cannot open a directory handle at all, so the bind
//! installs an FdReadEndpoint::Directory whose reads fabricate the
//! EISDIR failure into the fd table's read_error channel, and the
//! per-command read slurp records the same error for its fd-0 face.
//!
//! Documented residual: the external-child stdin face
//! (`cat < /proc/self/cwd`) keeps an open-error shape on Windows —
//! there is no directory handle to hand a child process; GNU's
//! `cat: -: Is a directory` is not reproducible there.
//!
//! Windows-gated: off Windows a real directory FileFd serves the
//! descriptor and fails its reads natively.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1937-{tag}-{}", std::process::id()));
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
fn issue1937_read_from_proc_self_cwd_reports_eisdir() {
    let dir = tempdir("read-direct");
    let run = rubash_c("read q < /proc/self/cwd; echo \"rc=$? q=[$q]\"", &dir);
    assert!(
        run.stderr.contains("read: 0: read error: Is a directory"),
        "stderr={}",
        run.stderr
    );
    assert!(run.stdout.contains("rc=1"), "stdout={}", run.stdout);
    assert!(run.stdout.contains("q=[]"), "stdout={}", run.stdout);
}

#[test]
fn issue1937_exec_bind_of_directory_succeeds_read_fails_eisdir() {
    let dir = tempdir("exec-bind");
    let run = rubash_c(
        "exec 3</proc/self/cwd; echo bound-ok; read -u 3 q; echo \"rc=$? q=[$q]\"; exec 3<&-",
        &dir,
    );
    assert!(
        run.stdout.contains("bound-ok"),
        "the bind itself must succeed: stdout={} stderr={}",
        run.stdout, run.stderr
    );
    assert!(
        run.stderr.contains("read: 3: read error: Is a directory"),
        "stderr={}",
        run.stderr
    );
    assert!(run.stdout.contains("rc=1"), "stdout={}", run.stdout);
    assert!(run.stdout.contains("q=[]"), "stdout={}", run.stdout);
}

#[test]
fn issue1937_compound_bind_of_directory_read_fails_eisdir() {
    let dir = tempdir("compound");
    let run = rubash_c("{ read -u 3 q; echo \"rc=$?\"; } 3</proc/self/cwd", &dir);
    assert!(
        run.stderr.contains("read: 3: read error: Is a directory"),
        "stderr={}",
        run.stderr
    );
    assert!(run.stdout.contains("rc=1"), "stdout={}", run.stdout);
}

#[test]
fn issue1937_content_entries_unaffected() {
    let dir = tempdir("control");
    let run = rubash_c("read -r l < /proc/self/status; echo \"l=$l\"", &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(run.stdout.contains("l=Name:"), "stdout={}", run.stdout);
}
