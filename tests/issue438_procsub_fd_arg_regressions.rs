//! Issue rubash#438 regressions: a process substitution passed as an
//! EXTERNAL command's argument must be readable by that command.
//!
//! GNU owner: execute_cmd.c execute_simple_command forks and execve's argv
//! unchanged; the child resolves `/dev/fd/N` because the shell's fork left
//! the substitution's pipe read end live at descriptor N (subst.c:6362
//! process_substitute -> move_to_high_fd -> make_dev_fd_filename). rubash's
//! virtual fd table is not the child's descriptor table, so the literal word
//! failed ENOENT (`diff: /dev/fd/63: No such file or directory`) — the pipe
//! end was never carried into the child.
//!
//! Fix: on Unix, materialize_dev_fd_operands backs each buffered
//! substitution endpoint with a real anonymous pipe and the spawn paths pin
//! the read end at its `/dev/fd/N` number between fork and exec
//! (attach_unix_dev_fd_operands pre_exec dup2), so the child resolves the
//! literal word through the OS /dev/fd layer exactly as a GNU fork.
//!
//! Unix-gated: on Windows the child cannot resolve a foreign fd at all and
//! the documented platform contract is temp-file materialization
//! (issue355_procsub_dev_fd_form_regressions covers that side).
//!
//! Black-box via CARGO_BIN_EXE_rubash; the byte contracts below are the
//! classic GNU semantics (verified shape against Git for Windows GNU bash
//! 5.2.37, msys build — WSL unavailable in the fix environment; #438 is a
//! longstanding fork/exec contract, not a 5.3 behavior).

#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn external_arg_reads_procsub_stream() {
    let (stdout, stderr, code) = rubash_stdin("cat <(echo hi)\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\n");
}

#[test]
fn pipeline_stage_reads_procsub_arg() {
    let (stdout, stderr, code) = rubash_stdin("cat <(echo hi) | head -1\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\n");
}

#[test]
fn diff_two_procsub_args_compare_equal() {
    // The issue's reproducer: real diff(1) opens /dev/fd/N twice.
    let (stdout, stderr, code) = rubash_stdin("diff <(echo a) <(echo a) && echo same\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "same\n");
}

#[test]
fn diff_two_procsub_args_report_difference() {
    let (stdout, stderr, code) = rubash_stdin("diff <(echo a) <(echo b) >/dev/null; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\n");
}

#[test]
fn two_args_read_independent_streams() {
    let (stdout, stderr, code) = rubash_stdin("cat <(echo left) <(echo right)\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "left\nright\n");
}

#[test]
fn payload_larger_than_pipe_buffer() {
    // The substitution content is fed from a writer thread: a payload well
    // past the 64 KiB kernel pipe buffer must not deadlock the shell.
    let (stdout, stderr, code) = rubash_stdin("cat <(seq 1 200000) | tail -1\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "200000\n");
}

#[test]
fn procsub_arg_inside_command_substitution() {
    let (stdout, stderr, code) = rubash_stdin("v=$(cat <(echo from-comsub))\necho v=$v\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "v=from-comsub\n");
}

#[test]
fn argv_word_stays_gnu_dev_fd_form() {
    // GNU prints the literal /dev/fd/N word (the child sees its own
    // descriptor); the fix must not rewrite argv to a temp path on Unix.
    let (stdout, stderr, code) = rubash_stdin("p=<(echo x)\ncat \"$p\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x\n");
}
