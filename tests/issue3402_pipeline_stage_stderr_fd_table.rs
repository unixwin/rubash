//! rubash#3402 — the Windows face of rubash#3308: a pipeline stage's
//! fd 2 bypassed the executor's fd table on the concurrent spawn paths.
//! GNU `execute_cmd.c:2620 execute_pipeline` forks every element with
//! the shell's fd 2 exactly as `redir.c:237 do_redirections` bound it,
//! so an enclosing `2>file` / `2>/dev/null` / `2>&-` / `exec 2>file`
//! binding IS each element's fd 2.
//!
//! #3308 wired the Unix concurrent twins; the Windows all-external
//! twin (`spawn_wait_concurrent_external_stages` + the raw
//! `std::io::stderr()` relay of the last stage), the Windows prefix
//! streaming tail, and the cross-platform streaming sandwich and
//! shell-writer consumer loops still left non-last members on the raw
//! process stderr. One cell per spawn path below.
//!
//! Expectations are the GNU 5.3.20 oracle's (WSL `/usr/local/bin/bash`,
//! probed from script files). Two GNU-correct "leaks" are pinned as
//! controls so the fix cannot over-suppress.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const LS_MISSING: &str = "nonexistent_xyz_probe";
const FN_G: &str = "g() { while read l; do echo \"g:$l\"; done; }\n";
const FN_F: &str = "f() { echo hi; }\n";

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3402-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

/// Run `script` as a script FILE under rubash with cwd = a fresh dir, so
/// `2>err.txt` targets stay relative (no host path quoting in the body).
/// Returns (stdout, stderr, status, err.txt contents).
fn run(name: &str, script: &str) -> (String, String, Option<i32>, String) {
    let dir = test_dir(name);
    fs::write(dir.join("probe.sh"), script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash script file");
    let err_file = fs::read_to_string(dir.join("err.txt")).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        err_file,
    )
}

/// The stage's diagnostic is silenced (`2>/dev/null` / `2>&-`).
fn assert_silenced(name: &str, script: &str, want_stdout: &str) {
    let (stdout, stderr, code, _) = run(name, script);
    assert_eq!(code, Some(0), "{name}: stderr: {stderr}");
    assert_eq!(stdout, want_stdout, "{name}: stderr: {stderr}");
    assert_eq!(stderr, "", "{name}: stage stderr bypassed the fd-2 binding");
}

/// The stage's diagnostic lands in `err.txt` and nowhere else.
fn assert_in_err_file(name: &str, script: &str, want_stdout: &str) {
    let (stdout, stderr, code, err_file) = run(name, script);
    assert_eq!(code, Some(0), "{name}: stderr: {stderr}");
    assert_eq!(stdout, want_stdout, "{name}: stderr: {stderr}");
    assert_eq!(stderr, "", "{name}: stage stderr bypassed 2>err.txt");
    assert!(
        err_file.contains(LS_MISSING),
        "{name}: err.txt missing the ls diagnostic: {err_file:?}"
    );
}

// --- all-external concurrent path (Windows twin of #3308) -------------

#[test]
fn concurrent_comsub_group_devnull() {
    assert_silenced(
        "cc-devnull",
        "{ cs=$(ls /nonexistent_xyz_probe | cat); } 2>/dev/null\necho \"len:${#cs}\"\n",
        "len:0\n",
    );
}

#[test]
fn concurrent_comsub_last_stage_devnull() {
    // The LAST stage fails: its collected stderr was relayed through the
    // raw process stderr.
    assert_silenced(
        "cc-last-devnull",
        "{ cs=$(cat | ls /nonexistent_xyz_probe); } 2>/dev/null </dev/null\necho \"len:${#cs}\"\n",
        "len:0\n",
    );
}

#[test]
fn concurrent_group_err_file() {
    assert_in_err_file(
        "c-errfile",
        "{ ls /nonexistent_xyz_probe | cat; } 2>err.txt\necho done\n",
        "done\n",
    );
}

#[test]
fn concurrent_group_closed_fd2() {
    assert_silenced(
        "c-closed",
        "{ ls /nonexistent_xyz_probe | cat; } 2>&-\necho done\n",
        "done\n",
    );
}

#[test]
fn concurrent_exec_err_file() {
    assert_in_err_file(
        "c-exec",
        "exec 2>err.txt\nls /nonexistent_xyz_probe | cat | cat\necho done\n",
        "done\n",
    );
}

#[test]
fn concurrent_merge_into_capture() {
    // `2>&1` inside the substitution: the diagnostic is captured.
    let (stdout, stderr, code, _) = run(
        "c-merge",
        "x=$( { ls /nonexistent_xyz_probe | cat | cat; } 2>&1 )\necho \"[$x]\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.starts_with('[') && stdout.contains(LS_MISSING) && stdout.ends_with("]\n"),
        "merged diagnostic not captured: {stdout:?}"
    );
    assert_eq!(stderr, "");
}

// --- prefix streaming tail (builtin tail) -----------------------------

#[test]
fn streaming_tail_err_file() {
    assert_in_err_file(
        "st-errfile",
        "{ ls /nonexistent_xyz_probe | cat | read l; } 2>err.txt\necho done\n",
        "done\n",
    );
}

#[test]
fn streaming_tail_devnull() {
    assert_silenced(
        "st-devnull",
        "{ ls /nonexistent_xyz_probe | read l; } 2>/dev/null\necho done\n",
        "done\n",
    );
}

// --- streaming sandwich (one shell-side stage) ------------------------

#[test]
fn sandwich_function_tail_devnull() {
    assert_silenced(
        "sw-devnull",
        &format!("{FN_G}{{ ls /nonexistent_xyz_probe | cat | g; }} 2>/dev/null\necho done\n"),
        "done\n",
    );
}

#[test]
fn sandwich_function_middle_err_file() {
    assert_in_err_file(
        "sw-errfile",
        &format!("{FN_G}{{ ls /nonexistent_xyz_probe | g | cat; }} 2>err.txt\necho done\n"),
        "done\n",
    );
}

#[test]
fn sandwich_closed_fd2() {
    assert_silenced(
        "sw-closed",
        &format!("{FN_G}{{ ls /nonexistent_xyz_probe | cat | g; }} 2>&-\necho done\n"),
        "done\n",
    );
}

// --- shell-writer (shell-side producer, external consumers) -----------

#[test]
fn shell_writer_consumer_devnull() {
    assert_silenced(
        "wr-devnull",
        &format!("{FN_F}{{ f | ls /nonexistent_xyz_probe | cat; }} 2>/dev/null\necho done\n"),
        "done\n",
    );
}

#[test]
fn shell_writer_consumer_err_file() {
    assert_in_err_file(
        "wr-errfile",
        &format!("{FN_F}{{ f | ls /nonexistent_xyz_probe | cat; }} 2>err.txt\necho done\n"),
        "done\n",
    );
}

#[test]
fn shell_writer_consumer_closed_fd2() {
    assert_silenced(
        "wr-closed",
        &format!("{FN_F}{{ f | ls /nonexistent_xyz_probe | cat; }} 2>&-\necho done\n"),
        "done\n",
    );
}

// --- external prefix ahead of two shell-side stages -------------------

#[test]
fn prefix_two_shell_stages_err_file() {
    assert_in_err_file(
        "px-errfile",
        &format!("{FN_G}{{ ls /nonexistent_xyz_probe | cat | g | g; }} 2>err.txt\necho done\n"),
        "done\n",
    );
}

#[test]
fn prefix_two_shell_stages_closed_fd2() {
    assert_silenced(
        "px-closed",
        &format!("{FN_G}{{ ls /nonexistent_xyz_probe | cat | g | g; }} 2>&-\necho done\n"),
        "done\n",
    );
}

// --- GNU-correct controls: no over-suppression ------------------------

#[test]
fn control_last_stage_redirect_does_not_silence_first_stage() {
    let (stdout, stderr, code, _) = run(
        "ctl-last",
        "ls /nonexistent_xyz_probe | cat 2>/dev/null\necho done\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n");
    assert!(
        stderr.contains(LS_MISSING),
        "first stage was silenced: {stderr:?}"
    );
}

#[test]
fn control_unredirected_sandwich_reaches_stderr() {
    let (stdout, stderr, code, _) = run(
        "ctl-plain",
        &format!("{FN_G}ls /nonexistent_xyz_probe | cat | g\necho done\n"),
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "done\n");
    assert!(stderr.contains(LS_MISSING), "diagnostic lost: {stderr:?}");
}

// --- rubash#3566: the stderr relay must not swallow the prefix stdout --

#[test]
fn prefix_stdout_survives_last_prefix_member_stderr() {
    // Two external prefix members, the last of which writes BOTH a
    // diagnostic and a payload line; two shell-side stages follow.
    let script = format!(
        "{FN_G}: > here.txt\nls . | ls here.txt /nonexistent_xyz_probe | g | g\necho done\n"
    );
    let (stdout, stderr, code, _) = run("px-payload", &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "g:g:here.txt\ndone\n", "stderr: {stderr}");
    assert!(stderr.contains(LS_MISSING), "diagnostic lost: {stderr:?}");
}

#[test]
fn prefix_stdout_survives_silenced_prefix_stderr() {
    let script = format!(
        "{FN_G}: > here.txt\n{{ ls . | ls here.txt /nonexistent_xyz_probe | g | g; }} 2>/dev/null\necho done\n"
    );
    let (stdout, stderr, code, _) = run("px-payload-devnull", &script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "g:g:here.txt\ndone\n", "stderr: {stderr}");
    assert_eq!(stderr, "");
}
