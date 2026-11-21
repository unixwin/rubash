//! Regression tests for rubash#3939: `<>` on `/dev/zero`, `/dev/null`, and `/dev/full`.
//!
//! GNU bash (`redir.c:725 redir_open`, `redir.c:924,949 do_redirection_internal`,
//! `make_cmd.c:682`) opens `<>` targets with `O_RDWR | O_CREAT` as a single open
//! file description shared by both read and write directions.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "rubash_issue3939_{tag}_{}_{}",
        std::process::id(),
        n
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn rubash_script(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    let script = dir.join("run.sh");
    std::fs::write(&script, body).expect("write script");
    let out = Command::new(RUBASH)
        .arg(&script)
        .current_dir(dir)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn issue3939_list_redirect_dev_zero_dup_write_succeeds() {
    let dir = scratch("list-zero");
    let (stdout, stderr, code) = rubash_script(&dir, "echo hi 3<>/dev/zero >&3\necho \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3939_list_redirect_dev_full_dup_write_enospc() {
    let dir = scratch("list-full");
    let (stdout, stderr, code) = rubash_script(&dir, "echo hi 3<>/dev/full >&3\necho \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("No space left on device"),
        "expected ENOSPC, got stderr: {stderr}"
    );
}

#[test]
fn issue3939_close_read_arm_closes_whole_rw_descriptor() {
    let dir = scratch("close-read");
    let (stdout, stderr, code) = rubash_script(
        &dir,
        "exec 3<>/dev/zero\necho first >&3\necho \"w1=$?\"\nexec 3<&-\necho second >&3\necho \"w2=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "w1=0\nw2=1\n");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected EBADF, got stderr: {stderr}"
    );
}

#[test]
fn issue3939_close_write_arm_closes_whole_rw_descriptor() {
    let dir = scratch("close-write");
    let (stdout, stderr, code) = rubash_script(
        &dir,
        "exec 3<>/dev/zero\nexec 3>&-\nread -r -t 0 <&3\necho \"r=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "r=1\n");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected EBADF, got stderr: {stderr}"
    );
}

#[test]
fn issue3939_dev_null_rw_write_and_eof_read() {
    let dir = scratch("dev-null");
    let (stdout, stderr, code) = rubash_script(
        &dir,
        "exec 3<>/dev/null\necho hi >&3\necho \"w=$?\"\nread -r x <&3\necho \"r=$?:$x\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "w=0\nr=1:\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3939_dynamic_fd_dev_zero_rw() {
    let dir = scratch("dyn-zero");
    let (stdout, stderr, code) = rubash_script(
        &dir,
        "exec {v}<>/dev/zero\necho hi >&\"$v\"\necho \"w=$?\"\nexec {v}>&-\necho after >&\"$v\"\necho \"after=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "w=0\nafter=1\n");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected EBADF, got stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn issue3939_unix_dev_zero_rw_single_open_file_description_in_child() {
    let dir = scratch("unix-child");
    let (stdout, stderr, code) = rubash_script(
        &dir,
        "exec 3<>/dev/zero\n/bin/sh -c 'echo hi >&3 && head -c 4 <&3 | od -An -tx1'\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, " 00 00 00 00\nrc=0\n");
    assert_eq!(stderr, "");
}

#[cfg(unix)]
#[test]
fn issue3939_unix_dynamic_fd_dev_zero_rw_single_open_file_description() {
    let dir = scratch("unix-dyn");
    // `{v}` is a shell variable, not exported — the child learns the
    // number through `$1`. /bin/sh is dash here, which cannot express
    // `>&10` (fds >= 10 are a syntax error), so the child is bash.
    // GNU 5.3 oracle on this exact snippet prints ` 00 00 00 00\nrc=0`.
    let (stdout, stderr, code) = rubash_script(
        &dir,
        "exec {v}<>/dev/zero\n/bin/bash -c 'echo hi >&$1 && head -c 4 <&$1 | od -An -tx1' _ \"$v\"\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, " 00 00 00 00\nrc=0\n");
    assert_eq!(stderr, "");
}
