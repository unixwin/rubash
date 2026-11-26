//! Issue unixwin/rubash#4044: an `exec` redirect walk inside a nested
//! context — a `( ... )` subshell, a `$( ... )` command substitution,
//! or a scoped (`builtin exec`) walk — retired the PROCESS's real
//! stdio descriptor along with the fd-table entry, so the outer
//! scope's later writes vanished: `( exec >&- 1>&10 ); echo after`
//! printed nothing, and `x=$(exec >&-; echo hi)` lost the whole
//! `x=[...]` line, not just the substitution's output.
//!
//! GNU runs every one of those bodies in a forked child
//! (execute_cmd.c:1576 execute_in_subshell; subst.c:7143
//! command_substitute) or undoes the scoped walk's redirects, so
//! r_close_this's `close(redirector)` (redir.c:1239, through
//! close_buffered_fd at redir.c:1267) drops the descriptor in the
//! CHILD's own descriptor space — the parent's fd 1 is untouched.
//! This executor runs those bodies in-process; the boundary restores
//! the fd TABLE only. The fix gates the kernel close in
//! `close_orphaned_process_fd` on the executor's process scope: fork
//! copies (`!owns_process_image`), in-process subshell layers above
//! the realized level (`subshell_depth > bashpid_origin_depth`),
//! emulated child regions, and scoped exec walks never land the
//! close; a respawned process at its realized level still does
//! (a coproc body's `exec >&-` must close the real c2p writer,
//! rubash#3633).
//!
//! Expectations verified against GNU bash 5.3.x (`/usr/local/bin/bash`,
//! script-file probes). The cases run the built binary, so they pin the
//! behaviour on every platform the fd model supports — on Windows no
//! kernel close exists and the same expectations hold there unchanged.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue4044-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
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

/// Run `body` as the script file `probe.sh` inside `dir` — the file
/// layout keeps the `probe.sh: line N:` diagnostic prefixes GNU
/// prints for its write-error diagnostics.
fn run_in(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    rubash_run(
        &[script.to_str().expect("script path utf8").to_string()],
        "",
    )
}

#[test]
fn issue4044_subshell_exec_close_then_dup_back() {
    // The issue's first reproducer: inside `( )` the exec walk saves
    // fd 1 at slot 10 and `1>&10` dups it back; GNU's child dies with
    // its close, so the parent's fd 1 stays live and `after` prints.
    let dir = test_dir("dupback");
    let (stdout, stderr, code) = run_in(&dir, "( exec >&- 1>&10 ); echo after\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4044_subshell_exec_close_stdout_survives() {
    let dir = test_dir("close1");
    let (stdout, stderr, code) = run_in(&dir, "( exec >&- ); echo after2\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after2\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4044_comsub_exec_close_parent_stdout_survives() {
    // `$( )`'s child closes its own fd 1, so the body's `echo hi`
    // meets a dead descriptor (GNU prints the write error on stderr)
    // — but the PARENT's fd 1 must survive to print `x=[]`.
    let dir = test_dir("comsub-close");
    let (stdout, stderr, code) = run_in(&dir, "x=$(exec >&-; echo hi); echo \"x=[$x]\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x=[]\n", "stderr: {stderr}");
    assert!(
        stderr.contains("echo: write error: Bad file descriptor"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue4044_comsub_exec_close_then_dup_back() {
    // Inside `$( )` the save at slot 10 dups back onto fd 1 — the
    // capture sees `hi` and the parent's fd 1 is unaffected.
    let dir = test_dir("comsub-dupback");
    let (stdout, stderr, code) = run_in(&dir, "x=$(exec >&- 1>&10; echo hi); echo \"x=[$x]\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x=[hi]\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4044_comsub_exec_close_only_parent_survives() {
    let dir = test_dir("comsub-close-only");
    let (stdout, stderr, code) = run_in(&dir, "x=$(exec >&-); echo \"x2=[$x]\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x2=[]\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4044_nested_subshell_layers() {
    // The inner `( )` is a second in-process layer: its close must
    // not reach the process's fd 1 for either enclosing scope.
    let dir = test_dir("nested");
    let (stdout, stderr, code) = run_in(&dir, "( ( exec >&- ); echo inner ); echo outer\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "inner\nouter\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4044_subshell_exec_close_stderr_survives() {
    let dir = test_dir("close2");
    let (stdout, stderr, code) = run_in(&dir, "( exec 2>&- ); echo e2back >&2\necho o\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "o\n", "stderr: {stderr}");
    assert_eq!(stderr, "e2back\n", "stdout: {stdout}");
}

#[test]
fn issue4044_builtin_exec_scoped_close_restored() {
    // `builtin exec`'s redirects are command-scoped — GNU undoes them
    // when the command returns, so fd 1 is live again for `after`.
    // The scoped walk's table-only restore cannot replay a kernel
    // close.
    let dir = test_dir("builtin-scoped");
    let (stdout, stderr, code) = run_in(&dir, "builtin exec >&-; echo after\necho rc=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "stdout: {stdout}");
}

#[test]
fn issue4044_toplevel_exec_close_still_persistent() {
    // The counter-face: a TOP-LEVEL `exec >&-` still drops the real
    // descriptor (GNU's close stands), so the next stdout write
    // reports the bad-fd diagnostic while fd 2 stays live.
    let dir = test_dir("persistent");
    let (stdout, stderr, code) = run_in(&dir, "exec >&-; echo hi; echo rc=$? >&2\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("echo: write error: Bad file descriptor\nrc=1\n"),
        "stderr: {stderr}"
    );
}
