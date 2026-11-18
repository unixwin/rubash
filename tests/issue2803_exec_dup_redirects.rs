//! rubash#2803 — `exec` with a command treated `>&fd` / `>&-` as
//! output to a file literally NAMED after the fd / dash:
//! `exec echo hi >&2` created a file `2` and sent nothing to stderr;
//! `exec echo hi >&-` created a file `-` instead of closing stdout.
//!
//! GNU redir.c do_redirection_internal (redir.c:803-870,
//! TRANSLATE_REDIRECT): for `[N]>&WORD` — WORD `-` is r_close_this,
//! WORD all-digits is r_duplicating_output (dup2 of that fd), and a
//! non-numeric WORD with redirector 1 and no varassign is
//! r_err_and_out (one open, fds 1 and 2 share it). The parser had
//! the kind right (`RedirectKind::DuplicateOutput`); the exec spawn
//! arm keyed on the raw `&…` mirror target and opened it as a file
//! name for EVERY `&…` target.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes (the issue's morph matrix mB10..mB20).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2803-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `body` as a script FILE under rubash with cwd = dir.
/// Returns (stdout, stderr).
fn rubash_script(dir: &Path, name: &str, body: &str) -> (String, String) {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script fixture");
    let output = Command::new(RUBASH)
        .arg(&script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn file_contents(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name)).ok()
}

#[test]
fn issue2803_exec_dup_out_to_stderr() {
    let dir = scratch("dup-to-stderr");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi >&2\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "hi\n");
    assert_eq!(
        file_contents(&dir, "2"),
        None,
        "no file named `2` may be created"
    );
}

#[test]
fn issue2803_exec_dup_fd1_to_stderr() {
    let dir = scratch("dup-fd1-to-stderr");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi 1>&2\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "hi\n");
    assert_eq!(
        file_contents(&dir, "2"),
        None,
        "no file named `2` may be created"
    );
}

#[test]
fn issue2803_exec_close_stdout_reports_write_error() {
    let dir = scratch("close-stdout");
    // Subshell form: GNU replaces the subshell's image, so the
    // status check must live in the parent shell.
    let (stdout, stderr) =
        rubash_script(&dir, "case.sh", "( exec echo hi >&- ); echo after rc=$?\n");
    assert_eq!(stdout, "after rc=1\n");
    assert!(
        stderr.contains("write error: Bad file descriptor"),
        "closed-stdout write must report EBADF, got stderr: {stderr:?}"
    );
    assert_eq!(
        file_contents(&dir, "-"),
        None,
        "no file named `-` may be created"
    );
}

#[test]
fn issue2803_exec_err_and_out_word_still_opens_file() {
    // Control: a non-numeric WORD is r_err_and_out in GNU — the one
    // `&…` shape the old arm classified correctly.
    let dir = scratch("err-and-out");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi >&log.txt\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "log.txt").as_deref(), Some("hi\n"));
}

#[test]
fn issue2803_exec_dup_failure_diagnostic_follows_fd2() {
    // The exec-failure diagnostic is an fd-2 byte: with `>&2` it
    // stays on the process stderr and no file `2` appears.
    let dir = scratch("dup-diag");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec rubash-no-such-command-2803 >&2\n");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("rubash-no-such-command-2803"),
        "exec failure diagnostic must reach stderr, got: {stderr:?}"
    );
    assert_eq!(
        file_contents(&dir, "2"),
        None,
        "no file named `2` may be created"
    );
}

#[test]
fn issue2803_bare_exec_dup_still_persists() {
    // Boundary control (both shells agree): bare `exec` with only
    // redirections uses the persistent walk, never the spawn arms.
    let dir = scratch("bare-exec-dup");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec >&2; echo hi\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "hi\n");
    assert_eq!(
        file_contents(&dir, "2"),
        None,
        "no file named `2` may be created"
    );
}
