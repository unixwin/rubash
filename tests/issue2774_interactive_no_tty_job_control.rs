//! rubash#2774 — an interactive shell that cannot establish job
//! control must not claim monitor mode, and must print GNU's two
//! initialization diagnostics.
//!
//! GNU: jobs.c:4731-4869 initialize_job_control actually tries to take
//! the terminal process group; on failure it reports
//! `cannot set terminal process group (%d)` (jobs.c:4844/4853) and
//! `no job control in this shell` (jobs.c:4858), and jobs.c:4869 makes
//! the `m` letter in `$-` the RESULT of that establishment
//! (`change_flag ('m', job_control ? '-' : '+')`) — an explicit `-m`
//! does not survive a terminal that cannot be taken.
//!
//! The child runs under setsid (its own session, no controlling
//! terminal) with piped stdio, so the no-tty shape is hermetic
//! regardless of the harness's own terminal. The pgrp number in the
//! first diagnostic varies per run and is normalized before comparing.
//!
//! Expectations verified against GNU bash 5.3.20 (piped stdin, no ctty).
//! Note: the last `$-` letter encodes the input source (shell.c
//! read_from_stdin: `s` for stdin, `c` for a -c string) and is asserted
//! only via the stable `hiBH` core plus the absence of `m`.

#![cfg(target_os = "linux")]

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

fn run_interactive(args: &[&str], input: Option<&str>) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    if let Some(input) = input {
        child
            .stdin
            .take()
            .expect("stdin pipe")
            .write_all(input.as_bytes())
            .expect("write stdin");
    } else {
        drop(child.stdin.take());
    }
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Replace the varying process-group number in the first diagnostic.
fn normalize_pgrp(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("cannot set terminal process group (") {
        out.push_str(&rest[..pos]);
        out.push_str("cannot set terminal process group (N)");
        let after = &rest[pos + "cannot set terminal process group (".len()..];
        let end = after.find(')').map(|i| i + 1).unwrap_or(after.len());
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn assert_no_job_control_shape(args: &[&str], input: Option<&str>, want_flags_line: bool) {
    let (stdout, stderr, code) = run_interactive(args, input);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    if want_flags_line {
        // The `$-` value is the only stdout content line consisting of
        // flag letters including `i`; it must carry the interactive
        // core letters and must NOT carry `m`.
        let flags = stdout
            .lines()
            .find(|line| line.contains('i') && line.chars().all(|c| c.is_ascii_alphabetic()))
            .unwrap_or_else(|| panic!("no $- line in stdout: {stdout:?}"));
        assert!(
            flags.contains("hiBH"),
            "$- must contain the interactive core hiBH: {flags:?}"
        );
        assert!(
            !flags.contains('m'),
            "$- must not claim monitor mode without job control: {flags:?}"
        );
    }
    let stderr = normalize_pgrp(&stderr);
    assert!(
        stderr.contains("cannot set terminal process group (N): Inappropriate ioctl for device"),
        "missing terminal-process-group diagnostic; stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("no job control in this shell"),
        "missing no-job-control diagnostic; stderr: {stderr:?}"
    );
}

#[test]
fn piped_interactive_shell_drops_monitor_and_reports() {
    assert_no_job_control_shape(&["-i"], Some("echo $-; exit\n"), true);
}

#[test]
fn devnull_interactive_shell_still_reports_at_startup() {
    // No commands at all: the diagnostics are an initialization-time
    // behavior, not tied to reading a first command.
    assert_no_job_control_shape(&["-i"], None, false);
}

#[test]
fn explicit_monitor_flag_does_not_survive_a_missing_terminal() {
    assert_no_job_control_shape(&["-i", "-m"], Some("echo $-; exit\n"), true);
}

#[test]
fn noninteractive_control_has_no_diagnostics() {
    let (stdout, stderr, code) = run_interactive(&[], Some("echo $-; exit\n"));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hBs\n");
    assert!(
        !stderr.contains("job control") && !stderr.contains("process group"),
        "non-interactive run must stay silent; stderr: {stderr:?}"
    );
}
