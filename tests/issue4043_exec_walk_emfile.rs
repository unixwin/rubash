//! Issue unixwin/rubash#4043: undo-save allocation at fd-table saturation.
//!
//! GNU's `add_undo_redirect` (`redir.c:1296`, `1303-1316`) materializes each
//! undo save via `fcntl(fd, F_DUPFD, ...)` (trying `fdbase + 1` /
//! `SHELL_FD_BASE`, then `SHELL_FD_BASE`, then `0`). When the fd table is
//! saturated under `ulimit -n` and no free slot remains, `add_undo_redirect`
//! calls `sys_error("redirection error: cannot duplicate fd")` (`EMFILE`:
//! `Too many open files`) and fails the redirection walk with status 1.
//!
//! Previously, `fd_walk_undo_save_slots` silently dropped the save when
//! `fd_walk_lowest_free` returned `None`, so undoable walks (`exec`,
//! builtins, compounds, functions) succeeded instead of failing `EMFILE`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue4043-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn run_script_file(dir: &Path, name: &str, body: &str) -> (String, String, Option<i32>) {
    let script = dir.join(name);
    std::fs::write(&script, body).expect("write script");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

#[test]
fn issue4043_exec_dup_emfile_at_saturation() {
    let dir = test_dir("exec-dup");
    let (stdout, stderr, code) = run_script_file(
        &dir,
        "probe.sh",
        "(\n\
           ulimit -n 32\n\
           for fd in $(seq 3 31); do\n\
             eval \"exec $fd>/dev/null\"\n\
           done\n\
           exec 1>&2\n\
           echo \"rc=$?\"\n\
         )\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("redirection error: cannot duplicate fd: Too many open files"),
        "expected EMFILE dup diagnostic, got: {stderr:?}"
    );
    assert!(
        stderr.contains("2: Too many open files"),
        "expected target Too many open files diagnostic, got: {stderr:?}"
    );
}

#[test]
fn issue4043_simple_builtin_dup_emfile_at_saturation() {
    let dir = test_dir("builtin-dup");
    let (stdout, stderr, code) = run_script_file(
        &dir,
        "probe.sh",
        "(\n\
           ulimit -n 32\n\
           for fd in $(seq 3 31); do\n\
             eval \"exec $fd>/dev/null\"\n\
           done\n\
           echo hi 1>&2\n\
           echo \"rc=$?\"\n\
         )\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("redirection error: cannot duplicate fd: Too many open files"),
        "expected EMFILE dup diagnostic, got: {stderr:?}"
    );
    assert!(
        !stderr.contains("hi\n"),
        "command body must not run on undo-save failure, got: {stderr:?}"
    );
}

#[test]
fn issue4043_exec_close_emfile_at_saturation() {
    let dir = test_dir("exec-close");
    let (stdout, stderr, code) = run_script_file(
        &dir,
        "probe.sh",
        "(\n\
           ulimit -n 32\n\
           for fd in $(seq 3 31); do\n\
             eval \"exec $fd>/dev/null\"\n\
           done\n\
           exec 3>&-\n\
           echo \"rc=$?\"\n\
         )\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("redirection error: cannot duplicate fd: Too many open files"),
        "expected EMFILE dup diagnostic, got: {stderr:?}"
    );
}

#[test]
fn issue4043_exec_open_consumes_last_slot_then_save_emfile() {
    let dir = test_dir("exec-open-last");
    let (stdout, stderr, code) = run_script_file(
        &dir,
        "probe.sh",
        "(\n\
           ulimit -n 32\n\
           for fd in $(seq 3 30); do\n\
             eval \"exec $fd>/dev/null\"\n\
           done\n\
           exec 1>/dev/null\n\
           echo \"rc=$?\"\n\
         )\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("redirection error: cannot duplicate fd: Too many open files"),
        "expected EMFILE dup diagnostic, got: {stderr:?}"
    );
    assert!(
        stderr.contains("/dev/null: Too many open files"),
        "expected /dev/null Too many open files diagnostic, got: {stderr:?}"
    );
}

#[test]
fn issue4043_compound_dup_emfile_at_saturation() {
    let dir = test_dir("compound-dup");
    let (stdout, stderr, code) = run_script_file(
        &dir,
        "probe.sh",
        "(\n\
           ulimit -n 32\n\
           for fd in $(seq 3 31); do\n\
             eval \"exec $fd>/dev/null\"\n\
           done\n\
           { echo hi; } 1>&2\n\
           echo \"rc=$?\"\n\
         )\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("redirection error: cannot duplicate fd: Too many open files"),
        "expected EMFILE dup diagnostic, got: {stderr:?}"
    );
}

#[test]
fn issue4043_subshell_head_active_only_succeeds_at_saturation() {
    let dir = test_dir("subshell-active");
    let (stdout, stderr, code) = run_script_file(
        &dir,
        "probe.sh",
        "(\n\
           ulimit -n 32\n\
           for fd in $(seq 3 31); do\n\
             eval \"exec $fd>/dev/null\"\n\
           done\n\
           ( echo hi ) 1>&2\n\
           echo \"rc=$?\"\n\
         )\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "hi\n", "stdout: {stdout}");
}
