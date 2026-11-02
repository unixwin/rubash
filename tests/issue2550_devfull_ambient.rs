//! rubash#2550 — an ambient (permanent `exec`) /dev/full binding was
//! bypassed for external children: `exec 1>/dev/full; /bin/echo hi`
//! handed the child the HOST stdout, so the write succeeded silently
//! (rc 0, no diagnostic). GNU installs the opened device in the
//! shell's own descriptor table (redir.c:237 do_redirections ->
//! redir.c:790 do_redirection_internal, the open at redir.c:725
//! redir_open) and the forked child inherits that descriptor: the
//! write fails ENOSPC inside the child, which diagnoses itself
//! (`/bin/echo: write error: No space left on device`) and exits 1.
//!
//! Expectations verified against GNU bash 5.3.20 (script files).

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2550-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(dir: &Path, name: &str, body: &str) -> (String, String) {
    let script = dir.join(name);
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(name)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

const WRITE_ERROR: &str = "/bin/echo: write error: No space left on device";

#[test]
fn ambient_fd1_full_device_reaches_external_child() {
    let dir = tempdir("fd1");
    let (stdout, stderr) = run_script(
        &dir,
        "probe.sh",
        "exec 3>&1\n\
         exec 1>/dev/full\n\
         /bin/echo hi\n\
         echo \"rc=$?\" >&3\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(stderr.contains(WRITE_ERROR), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ambient_fd1_append_full_device_reaches_external_child() {
    let dir = tempdir("fd1app");
    let (stdout, stderr) = run_script(
        &dir,
        "probe.sh",
        "exec 3>&1\n\
         exec 1>>/dev/full\n\
         /bin/echo hi\n\
         echo \"rc=$?\" >&3\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(stderr.contains(WRITE_ERROR), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ambient_fd2_full_device_reaches_external_child() {
    let dir = tempdir("fd2");
    // The child's fd 1 dups the ambient fd 2 binding; its fd 2 is
    // restored to the saved stderr so the child's own diagnostic
    // stays observable.
    let (stdout, stderr) = run_script(
        &dir,
        "probe.sh",
        "exec 3>&2\n\
         exec 2>/dev/full\n\
         /bin/echo hi 1>&2 2>&3\n\
         echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(stderr.contains(WRITE_ERROR), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ambient_fd1_full_device_merged_stderr_dup() {
    let dir = tempdir("merge");
    // `2>&1` dups the ambient /dev/full binding: the payload write
    // and the child's diagnostic both land on the full device, so
    // nothing is observable but the status.
    let (stdout, stderr) = run_script(
        &dir,
        "probe.sh",
        "exec 3>&1\n\
         exec 1>/dev/full\n\
         /bin/echo hi 2>&1\n\
         echo \"rc=$?\" >&3\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(!stderr.contains("hi"), "stderr: {stderr}");
    assert!(!stderr.contains("write error"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ambient_high_fd_full_device_dup_source() {
    let dir = tempdir("fd4");
    let (stdout, stderr) = run_script(
        &dir,
        "probe.sh",
        "exec 3>&1\n\
         exec 4>/dev/full\n\
         /bin/echo hi >&4\n\
         echo \"rc=$?\" >&3\n",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(stderr.contains(WRITE_ERROR), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ambient_dev_null_control_stays_silent_success() {
    let dir = tempdir("null");
    let (stdout, stderr) = run_script(
        &dir,
        "probe.sh",
        "exec 3>&1\n\
         exec 1>/dev/null\n\
         /bin/echo hi\n\
         echo \"rc=$?\" >&3\n",
    );
    assert_eq!(stdout, "rc=0\n");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ambient_regular_file_control_still_delivers() {
    let dir = tempdir("file");
    let (stdout, stderr) = run_script(
        &dir,
        "probe.sh",
        "exec 3>&1\n\
         exec 1>out.txt\n\
         /bin/echo hi\n\
         echo \"rc=$?\" >&3\n",
    );
    assert_eq!(stdout, "rc=0\n");
    assert!(stderr.is_empty(), "stderr: {stderr}");
    let delivered = std::fs::read_to_string(dir.join("out.txt")).expect("read out.txt");
    assert_eq!(delivered, "hi\n");
    let _ = std::fs::remove_dir_all(&dir);
}
