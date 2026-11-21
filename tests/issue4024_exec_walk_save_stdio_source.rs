//! Issue unixwin/rubash#4024: an exec walk's undo save of a stdio
//! descriptor materialized as a clone of the symbolic endpoint —
//! `Stdout` / `Stderr` / `InheritedProcessStdin` are ALIASES of
//! process descriptors 1/2/0, so the same list's `>&-` / `<&-`
//! retired the real descriptor and a later `N>&<slot>` duped back a
//! dead endpoint: `exec >&- 1>&10; echo hi` printed nothing, no
//! diagnostic, rc 0.
//!
//! GNU's save is a REAL descriptor: add_undo_redirect
//! (redir.c:1296) runs `fcntl (fd, F_DUPFD, SHELL_FD_BASE)`
//! (redir.c:1303) before the entry binds, so the save survives the
//! redirector's own close and the dup-back binds a live endpoint.
//! The fix materializes the exec walk's saves through
//! `FdTable::exec_walk_saved_entry`, which upgrades the symbolic
//! stdio arms to private descriptor duplicates instead of cloning
//! the alias.
//!
//! These cases run the built binary, so they pin the behaviour on
//! every platform the fd model supports — on Unix the symbolic
//! clone died with the kernel close (close_orphaned_process_fd,
//! rubash#3930); on Windows the alias kept resolving, so the same
//! expectations hold there unchanged.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue4024-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    rubash_run(&[], script)
}

/// Run `body` as a script inside `dir`.
fn run_in(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    // Single-quote the path: on Windows it carries backslashes the
    // script's own quoting rules would otherwise strip.
    rubash_stdin(&format!("cd '{}' || exit 1\n{body}", dir.display()))
}

/// Run rubash on a script file with `stdin_payload` on the process's
/// real fd 0 — the input-side cases need fd 0 to be a payload stream,
/// which only a script FILE leaves free (a stdin script occupies it).
fn run_script_file(
    dir: &Path,
    name: &str,
    body: &str,
    stdin_payload: &str,
) -> (String, String, Option<i32>) {
    let script = dir.join(name);
    std::fs::write(&script, body).expect("write script");
    rubash_run(
        &[script.to_str().expect("script path utf8").to_string()],
        stdin_payload,
    )
}

fn rubash_run(args: &[String], stdin_payload: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(stdin_payload.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
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
fn issue4024_exec_close_then_dup_back_stdout() {
    // The issue's minimal reproducer: `>&-` earns fd 1's undo save
    // at slot 10 (add_undo_redirect, redir.c:1253), then `1>&10`
    // dups it back — fd 1 must be live again.
    let dir = test_dir("out");
    let (stdout, stderr, code) = run_in(&dir, "exec >&- 1>&10\necho hi\necho rc=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4024_exec_save_feeds_stderr_from_stdout_save() {
    // `1>&-` saves fd 1 at slot 10; `2>&10` binds fd 2 to the
    // SAVED stdout description, so a write to fd 2 lands on what
    // fd 1 used to name.
    let dir = test_dir("err-from-out");
    let (stdout, stderr, code) = run_in(&dir, "exec 1>&- 2>&10\necho C:to-stderr >&2\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "C:to-stderr\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4024_exec_close_stderr_dup_back_to_stdout() {
    // `2>&-` saves fd 2 at slot 10; `1>&10` binds fd 1 to the
    // saved STDERR description — writes to fd 1 land on stderr,
    // and a write to the still-closed fd 2 fails.
    let dir = test_dir("out-from-err");
    let (stdout, stderr, code) = run_in(&dir, "exec 2>&- 1>&10\necho J:out\necho J:err >&2\n");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(stderr, "J:out\n", "stdout: {stdout}");
    assert_eq!(code, Some(1));
}

#[test]
fn issue4024_exec_stderr_restored_carries_diagnostics() {
    // `1>&-` saves fd 1 at slot 10; `2>&10` binds fd 2 to the
    // saved stdout — the write-error diagnostic for the closed
    // fd 1 reports through fd 2 and lands on stdout (GNU shape).
    let dir = test_dir("diag");
    let (stdout, stderr, code) = run_in(&dir, "exec 1>&- 2>&10\necho K:out\necho K:err >&2\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("write error: Bad file descriptor"),
        "fd-2 diagnostic must reach the restored stdout; got {stdout:?}"
    );
    assert!(
        stdout.contains("K:err"),
        "fd-2 write must land on the saved stdout; got {stdout:?}"
    );
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4024_exec_stdin_close_then_dup_back() {
    // The input face: `<&-` earns fd 0's undo save at slot 10,
    // `0<&10` dups it back — a `read` then consumes the process's
    // real stdin stream through the restored descriptor. Run as a
    // script FILE so fd 0 is a payload pipe, not the script.
    let dir = test_dir("stdin");
    let (stdout, stderr, code) = run_script_file(
        &dir,
        "in.sh",
        "exec <&- 0<&10\nread x\necho \"got=[$x]\"\n",
        "hello\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "got=[hello]\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4024_exec_save_disposed_when_list_stands() {
    // The walk's saves are walk-local currency — once the exec
    // list stands they are disposed (execute_cmd.c:5619 disposes
    // the undo list), so a later `>&10` finds no descriptor.
    let dir = test_dir("disposed");
    let (stdout, stderr, code) = run_in(
        &dir,
        "exec >&- 1>&10\necho hi\n( echo x >&10 ) 2>/dev/null\necho leak_rc=$?\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\nleak_rc=1\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4024_no_save_for_unopened_redirector() {
    // Boundary pin: fd 3 was never open, so `3>&-` registers no
    // descriptor save (add_undo_close_redirect, redir.c:1255) —
    // the `4>&10` dup fails EBADF exactly like GNU, and the exec
    // walk stops there.
    let dir = test_dir("no-save");
    let (stdout, stderr, code) = run_in(&dir, "exec 3>&- 4>&10\necho D:after\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "D:after\n", "stderr: {stderr}");
    assert!(
        stderr.contains("10: Bad file descriptor"),
        "stderr must name the bad dup source; got {stderr:?}"
    );
}
