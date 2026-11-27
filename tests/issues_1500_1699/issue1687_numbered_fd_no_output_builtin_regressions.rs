//! Issue rubash#1687: numbered fd redirections (`3>file`) were silently
//! skipped for the no-output builtins — bare `builtin`, `:` (and the
//! `true`/`false` siblings) — because the shared helper
//! `apply_no_output_builtin_redirects_with_status` only walked the
//! stdio fields (fds 0-2) and never the numbered entries of
//! `cmd.redirects`. GNU performs every redirection on the command node
//! (redir.c do_redirections): the files are created/truncated, a bad
//! target fails the builtin with status 1, and a dup from a closed fd
//! fails EBADF — even though these builtins write nothing themselves.
//! Expectations below were measured against GNU bash 5.3.20 with the
//! same scripts.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1687-{tag}-{}", std::process::id()));
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
    }
}

#[test]
fn issue1687_colon_numbered_fd_creates_file() {
    let dir = tempdir("colon-create");
    let run = rubash_c(
        ": 3>g.txt; test -e g.txt && echo g_exists || echo g_missing",
        &dir,
    );
    assert_eq!(run.stdout, "g_exists\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1687_numbered_fd_truncates_existing_file() {
    let dir = tempdir("truncate");
    std::fs::write(dir.join("h.txt"), "old\n").expect("seed file");
    let run = rubash_c(": 3>h.txt; stat -c %s h.txt", &dir);
    assert_eq!(run.stdout, "0\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1687_numbered_fd_append_keeps_existing_content() {
    let dir = tempdir("append");
    std::fs::write(dir.join("i.txt"), "old\n").expect("seed file");
    let run = rubash_c("builtin 3>>i.txt; cat i.txt", &dir);
    assert_eq!(run.stdout, "old\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1687_numbered_fd_bad_path_fails_builtin() {
    let dir = tempdir("bad-path");
    let run = rubash_c(": 3>/no/such/dir/x.txt; echo rc=$?", &dir);
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
    let run = rubash_c("builtin 3>/no/such/dir/x.txt; echo rc=$?", &dir);
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1687_numbered_fd_input_missing_fails_existing_opens() {
    let dir = tempdir("input");
    let run = rubash_c(": 3<seed.txt; echo rc=$?", &dir);
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
    std::fs::write(dir.join("seed.txt"), "x\n").expect("seed file");
    let run = rubash_c(": 3<seed.txt; echo rc=$?", &dir);
    assert_eq!(run.stdout, "rc=0\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1687_numbered_fd_dup_validates_source() {
    let dir = tempdir("dup");
    // Closed source: GNU fails the dup with EBADF (status 1). The
    // `exec 9>&-` first makes "closed" deterministic: an inherited open
    // fd 9 (the test runner's own pipes) would be a legitimate dup
    // source (rubash#1557 inherited-fd semantics; GNU agrees there).
    let run = rubash_c("exec 9>&-; : 3>&9; echo rc=$?", &dir);
    assert_eq!(run.stdout, "rc=1\n", "stderr: {}", run.stderr);
    // Open source: the dup succeeds.
    let run = rubash_c("exec 9>/dev/null; : 3>&9; echo rc=$?", &dir);
    assert_eq!(run.stdout, "rc=0\n", "stderr: {}", run.stderr);
}

#[test]
fn issue1687_true_false_numbered_fd_creates_file() {
    let dir = tempdir("true-false");
    let run = rubash_c(
        "true 3>j.txt; echo true_rc=$?; false 3>k.txt; echo false_rc=$?; \
         test -e j.txt && echo j_exists; test -e k.txt && echo k_exists",
        &dir,
    );
    assert_eq!(
        run.stdout, "true_rc=0\nfalse_rc=1\nj_exists\nk_exists\n",
        "stderr: {}",
        run.stderr
    );
}
