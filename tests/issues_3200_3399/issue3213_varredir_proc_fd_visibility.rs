//! rubash#3213 — a varredir (`{var}`) materialized fd must be a REAL
//! descriptor in the shell process at its number, visible in the
//! shell's own /proc/$$/fd view, exactly like GNU bash 5.3.20.
//!
//! GNU opens the file, then `fcntl(fd, F_DUPFD, SHELL_FD_BASE)`
//! dups it to the first free number >= 10 (redir.c:963-968, the
//! file-open arms of do_redirection_internal), binds the variable
//! to that number (redir.c:1470-1482 redir_varassign), and closes
//! the low original (redir.c:1040-1047) — so /proc/$$/fd holds the
//! number and readlink resolves to the target file. rubash kept the
//! fd virtual (an fd-table entry whose FileFd raw handle sat at some
//! other low number), materialized into children only at spawn
//! through the pin pass — the shell's own /proc/$$/fd lacked the
//! number while exec-form children saw it.
//!
//! Expectations verified against GNU bash 5.3.20 with identical
//! script files (fresh process, stdin null). Linux-only: the
//! assertions read /proc/$$/fd, which does not exist on macOS or
//! Windows.

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3213-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(dir: &PathBuf, text: &str) -> (String, String, Option<i32>) {
    let script = dir.join("probe.sh");
    std::fs::write(&script, text).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue3213_output_varredir_fd_visible_in_own_proc_view() {
    let dir = tempdir("output");
    let (stdout, stderr, code) = run_script(
        &dir,
        "{v}>f.txt echo hi\necho \"v=$v\"\nls /proc/$$/fd | grep -qx \"$v\"; echo \"has=$?\"\ntest -e /proc/$$/fd/$v; echo \"exists=$?\"\nreadlink /proc/$$/fd/$v | grep -q \"f\\.txt$\"; echo \"rl=$?\"\n",
    );
    assert_eq!(
        stdout, "hi\nv=10\nhas=0\nexists=0\nrl=0\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3213_input_varredir_fd_visible() {
    // NOTE: a bare `{v}<f.txt` (no command) binds nothing in GNU — the
    // redirection is set up and torn down with the null command — so
    // the exec form carries the binding here (verified: GNU prints
    // v= empty for the bare form, v=10 for exec {v}<f.txt).
    let dir = tempdir("input");
    let (stdout, stderr, code) = run_script(
        &dir,
        "echo data > f.txt\nexec {v}<f.txt\necho \"v=$v\"\nls /proc/$$/fd | grep -qx \"$v\"; echo \"has=$?\"\nreadlink /proc/$$/fd/$v | grep -q \"f\\.txt$\"; echo \"rl=$?\"\ncat <&$v\n",
    );
    assert_eq!(stdout, "v=10\nhas=0\nrl=0\ndata\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3213_dup_varredir_fd_visible_and_shared() {
    // NOTE: same bare-form caveat as the input test: `exec {w}>g.txt`
    // opens the persistent descriptor that `{v}>&$w` then dups.
    let dir = tempdir("dup");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec {w}>g.txt\n{v}>&$w echo hi\necho \"v=$v w=$w\"\nls /proc/$$/fd | grep -qx \"$v\"; echo \"has=$?\"\nreadlink /proc/$$/fd/$v | grep -q \"g\\.txt$\"; echo \"rl=$?\"\necho more >&$v\ncat g.txt\n",
    );
    assert_eq!(
        stdout, "hi\nv=11 w=10\nhas=0\nrl=0\nmore\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3213_close_removes_number() {
    let dir = tempdir("close");
    let (stdout, stderr, code) = run_script(
        &dir,
        "{v}>f.txt echo hi\nexec {v}>&-\nls /proc/$$/fd | grep -qx \"$v\"; echo \"has=$?\"\ntest -e /proc/$$/fd/$v; echo \"exists=$?\"\n",
    );
    assert_eq!(stdout, "hi\nhas=1\nexists=1\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3213_option_on_closes_at_command_end() {
    // Control (mirrors issue2775_option_on_still_closes): with
    // varredir_close ON the descriptor is closed at command end, so
    // the number must already be gone from /proc/$$/fd afterwards.
    let dir = tempdir("option-on");
    let (stdout, stderr, code) = run_script(
        &dir,
        "shopt -s varredir_close\n{v}>f.txt echo hi\nls /proc/$$/fd | grep -qx \"$v\"; echo \"has=$?\"\n",
    );
    assert_eq!(stdout, "hi\nhas=1\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3213_exec_form_child_still_inherits() {
    // No-regression guard (passes on base too): exec-form children
    // already saw the varredir fd through the spawn pin pass; the
    // materialization must not change that.
    let dir = tempdir("exec-child");
    let (stdout, stderr, code) = run_script(&dir, "{v}>f.txt /bin/ls /proc/self/fd\n");
    assert!(
        stdout.lines().any(|line| line.trim() == "10"),
        "child fd list should contain 10: {stdout:?} stderr: {stderr}"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3213_fixed_high_fd_visible() {
    // Same mechanism, fixed form: a persistent fd >= SHELL_FD_BASE
    // is a real descriptor in the shell process too (GNU opens it
    // on that number); the fd-table materialization covers it.
    let dir = tempdir("fixed-high");
    let (stdout, stderr, code) = run_script(
        &dir,
        "exec 12>f.txt\nls /proc/$$/fd | grep -qx 12; echo \"has=$?\"\nreadlink /proc/$$/fd/12 | grep -q \"f\\.txt$\"; echo \"rl=$?\"\nexec 12>&-\n",
    );
    assert_eq!(stdout, "has=0\nrl=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
