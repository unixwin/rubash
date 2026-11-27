//! rubash#1605 (residual of rubash#1454): a brace group (or function)
//! in the FINAL pipeline position whose fd 1 is the shell's own stdout
//! must run with that stdout inherited, not behind a capture pipe —
//! GNU runs the element with the inherited fd 1, so
//! `echo hello | { readlink /proc/self/fd/1; }` names the shell's
//! stdout target while rubash printed `pipe:[...]` (a fresh pipe).
//!
//! Oracle: GNU bash 5.3.20, script file, shell stdout redirected to a
//! regular file. Both the group form and the bare-group control print
//! the file's path; a NON-final group still sees the pipe.

#![cfg(unix)]

use std::process::{Command, Stdio};

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// Run `script` with the shell's stdout redirected to `<dir>/out.txt`
/// and return (out.txt contents, stderr, other.txt contents if any).
fn run_with_stdout_to_file(script: &str) -> (String, String, Option<String>) {
    let dir = scratch_tempdir();
    let script_path = dir.path().join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let out_path = dir.path().join("out.txt");
    let out_file = std::fs::File::create(&out_path).expect("create out");
    let output = Command::new(rubash_bin())
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", dir.path())
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let stdout = std::fs::read_to_string(&out_path).expect("read out");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let other = std::fs::read_to_string(dir.path().join("other.txt")).ok();
    (stdout, stderr, other)
}

#[test]
fn issue1605_final_brace_group_inherits_shell_stdout() {
    let (stdout, stderr, _) = run_with_stdout_to_file(
        "echo hello | { readlink /proc/self/fd/1 || echo nolink; }\n\
         { readlink /proc/self/fd/1 || echo nolink; }\n",
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout: {stdout:?} stderr: {stderr:?}");
    // The group line must equal the bare-group control line (the
    // shell's own fd 1) and must not be a fresh pipe.
    assert_eq!(lines[0], lines[1], "stdout: {stdout:?}");
    assert!(
        lines[0].ends_with("out.txt"),
        "group fd 1 should be the redirected file, got: {}",
        lines[0]
    );
    assert!(!lines[0].starts_with("pipe:["), "stdout: {stdout:?}");
}

#[test]
fn issue1605_final_function_stage_inherits_shell_stdout() {
    let (stdout, stderr, _) = run_with_stdout_to_file(
        "f() { readlink /proc/self/fd/1 || echo nolink; }\n\
         echo x | f\n",
    );
    let line = stdout.trim_end();
    assert!(
        line.ends_with("out.txt"),
        "function fd 1 should be the redirected file, got stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert!(!line.starts_with("pipe:["), "stdout: {stdout:?}");
}

#[test]
fn issue1605_non_final_group_still_sees_the_pipe() {
    let (stdout, stderr, _) = run_with_stdout_to_file(
        "echo hello | { readlink /proc/self/fd/1 || echo nolink; } | cat\n",
    );
    let line = stdout.trim_end();
    assert!(
        line.starts_with("pipe:["),
        "non-final group fd 1 must stay the pipe, got stdout: {stdout:?} stderr: {stderr:?}"
    );
}

#[test]
fn issue1605_final_group_with_own_redirect_keeps_it() {
    let (stdout, stderr, other) = run_with_stdout_to_file(
        "echo hello | { readlink /proc/self/fd/1 || echo nolink; } > other.txt\n",
    );
    // The group's own redirect rebinds fd 1 for the body: readlink's
    // output lands IN other.txt, so the shell's stdout stays empty and
    // other.txt names other.txt as the fd-1 target.
    assert_eq!(stdout, "", "stdout: {stdout:?} stderr: {stderr:?}");
    let other = other.expect("other.txt should exist");
    assert!(
        other.trim_end().ends_with("other.txt"),
        "other.txt: {other:?}"
    );
}
