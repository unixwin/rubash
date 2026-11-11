//! rubash#2821 — `exec` with a command applied only the lowest
//! numbered output redirection and silently dropped the rest: the
//! other targets were never even created, and a failing command's
//! diagnostic landed on the original stderr instead of the file its
//! `2>` named.
//!
//! GNU redir.c do_redirection_internal (redir.c:767+) walks the
//! command's whole redirect list left to right before exec_builtin
//! (builtins/exec.def:231) hands the bound descriptors to
//! shell_execve (execute_cmd.c:6126): every output target is opened
//! in list order, and each fd's binding is whatever the walk last
//! assigned it (a dup resolves against the bindings the walk has
//! already made). rubash's exec spawn consulted only the scalar
//! mirror fields (redirect_out / append / redirect_err /
//! redirect_err_append) — the first matching arm won and returned,
//! so at most one redirection ever took effect.
//!
//! Expectations byte-verified against GNU bash 5.3.20 with
//! script-file probes (the issue's matrix, cells ex-01..ex-18).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2821-{tag}-{}", std::process::id()));
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
fn issue2821_exec_out_then_err_both_apply() {
    let dir = scratch("out-err");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi >o.txt 2>e.txt\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "o.txt").as_deref(), Some("hi\n"));
    assert_eq!(file_contents(&dir, "e.txt").as_deref(), Some(""));
}

#[test]
fn issue2821_exec_err_then_out_both_apply() {
    let dir = scratch("err-out");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi 2>e.txt >o.txt\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "o.txt").as_deref(), Some("hi\n"));
    assert_eq!(file_contents(&dir, "e.txt").as_deref(), Some(""));
}

#[test]
fn issue2821_exec_append_out_then_err_both_apply() {
    let dir = scratch("append-out-err");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi >>o.txt 2>e.txt\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "o.txt").as_deref(), Some("hi\n"));
    assert_eq!(file_contents(&dir, "e.txt").as_deref(), Some(""));
}

#[test]
fn issue2821_exec_out_then_append_err_both_apply() {
    let dir = scratch("out-append-err");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi >o.txt 2>>e.txt\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "o.txt").as_deref(), Some("hi\n"));
    assert_eq!(file_contents(&dir, "e.txt").as_deref(), Some(""));
}

#[test]
fn issue2821_exec_numbered_fd_target_is_created() {
    let dir = scratch("numbered-first");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi 3>f.txt >o.txt\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "o.txt").as_deref(), Some("hi\n"));
    assert_eq!(file_contents(&dir, "f.txt").as_deref(), Some(""));
}

#[test]
fn issue2821_exec_err_and_numbered_targets_are_created() {
    let dir = scratch("err-and-numbered");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi 2>e.txt 3>f.txt\n");
    assert_eq!(stdout, "hi\n");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "e.txt").as_deref(), Some(""));
    assert_eq!(file_contents(&dir, "f.txt").as_deref(), Some(""));
}

#[test]
fn issue2821_exec_failure_diagnostic_lands_in_err_file() {
    let dir = scratch("failure-diag");
    let (stdout, stderr) = rubash_script(
        &dir,
        "case.sh",
        "exec rubash-no-such-command-2821 >o.txt 2>e.txt\n",
    );
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "o.txt").as_deref(), Some(""));
    let diag = file_contents(&dir, "e.txt").expect("err.txt must be created");
    assert!(
        diag.contains("rubash-no-such-command-2821"),
        "exec failure diagnostic must land in e.txt, got: {diag:?}"
    );
}

#[test]
fn issue2821_exec_single_err_redirect_still_applies() {
    // Control: the one shape the old arm chain already handled.
    let dir = scratch("single-err");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi 2>e.txt\n");
    assert_eq!(stdout, "hi\n");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "e.txt").as_deref(), Some(""));
}

#[test]
fn issue2821_exec_dup_err_after_out_shares_the_file() {
    // Control: `>o 2>&1` — fd 2 dups the walk's fd-1 binding.
    let dir = scratch("out-then-dup");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi >o.txt 2>&1\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "o.txt").as_deref(), Some("hi\n"));
}

#[test]
fn issue2821_exec_dup_err_before_out_keeps_pipe() {
    // Control: `2>&1 >o` — fd 2 dups fd 1's at-position binding (the
    // pipe), so nothing reaches the terminal stderr either.
    let dir = scratch("dup-then-out");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi 2>&1 >o.txt\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "o.txt").as_deref(), Some("hi\n"));
}

#[test]
fn issue2821_exec_close_err_then_out() {
    // Control: `2>&- >o` already agreed with GNU on the base.
    let dir = scratch("close-err-then-out");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec echo hi 2>&- >o.txt\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "o.txt").as_deref(), Some("hi\n"));
}

#[test]
fn issue2821_exec_input_and_out_both_apply() {
    // Control: fd-0 input redirects were never part of the defect.
    let dir = scratch("input-and-out");
    fs::write(dir.join("in.txt"), "data\n").expect("write input fixture");
    let (stdout, stderr) = rubash_script(&dir, "case.sh", "exec cat <in.txt >o.txt\n");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(file_contents(&dir, "o.txt").as_deref(), Some("data\n"));
}
