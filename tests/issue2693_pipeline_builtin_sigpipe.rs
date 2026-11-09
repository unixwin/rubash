//! rubash#2693 — pipeline-stage builtins are SIGPIPE-immune.
//!
//! GNU ground truth (bash 5.3.20 oracle, probed from script files):
//! a pipeline stage is a real child (jobs.c:2265 make_child), and
//! the child resets signal dispositions before running the stage —
//! trap.c:1480 reset_signal_handlers, called on the forked-child
//! path at execute_cmd.c:1670 — so an UNTRAPPED SIGPIPE is SIG_DFL
//! in the stage and a builtin producer whose consumer closes early
//! DIES by SIGPIPE: PIPESTATUS reports 141. rubash starts with
//! SIGPIPE ignored (Rust runtime) and its forkless stage-emulation
//! entry never restores SIG_DFL for builtin stages, so the stage
//! completes and PIPESTATUS reports 0.
//!
//! The external-stage and small-write cells are controls: external
//! children already die with 141 (std::process resets SIGPIPE for
//! spawned children), and a write that fits the pipe buffer before
//! the consumer exits raises no SIGPIPE in either shell.

#![cfg(unix)]

use std::process::{Command, Stdio};

fn run_rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's reproducer verbatim: a builtin `printf` producing
/// ~400 KB into a pipe whose reader exits after one line.
#[test]
fn printf_stage_dies_by_sigpipe() {
    let (stdout, stderr, code) = run_rubash(
        "printf 'y\\n%.0s' $(seq 200000) | head -1\n\
         echo \"ps0=${PIPESTATUS[0]} ps1=${PIPESTATUS[1]}\"",
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "y\nps0=141 ps1=0\n", "stderr: {stderr}");
}

/// A builtin loop stage: the stage's `echo` writes must meet the
/// same fate once `head` closes the pipe.
#[test]
fn echo_loop_stage_dies_by_sigpipe() {
    let (stdout, stderr, code) = run_rubash(
        "for i in $(seq 1 200000); do echo \"line$i\"; done | head -1\n\
         echo \"ps0=${PIPESTATUS[0]} ps1=${PIPESTATUS[1]}\"",
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "line1\nps0=141 ps1=0\n", "stderr: {stderr}");
}

/// Control (regression lock): an external producer already dies by
/// SIGPIPE in both shells.
#[test]
fn external_stage_sigpipe_control() {
    let (stdout, stderr, code) = run_rubash(
        "seq 200000 | head -1\n\
         echo \"ext ps0=${PIPESTATUS[0]} ps1=${PIPESTATUS[1]}\"",
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "1\next ps0=141 ps1=0\n", "stderr: {stderr}");
}

/// Control (regression lock): a builtin write that fits the pipe
/// buffer completes before the consumer exits — no SIGPIPE, the
/// stage reports 0 in GNU too.
#[test]
fn small_builtin_write_no_sigpipe() {
    let (stdout, stderr, code) = run_rubash(
        "printf 'y\\n' | head -1\n\
         echo \"small ps0=${PIPESTATUS[0]} ps1=${PIPESTATUS[1]}\"",
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "y\nsmall ps0=0 ps1=0\n", "stderr: {stderr}");
}
