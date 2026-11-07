//! rubash#2925: `kill -INT` cannot kill a non-monitored async job.
//!
//! GNU ground truth (bash 5.3.20, execute_cmd.c:5850-5859
//! execute_disk_command / setup_async_signals at :5709): the async
//! child installs the SIGINT/SIGQUIT ignore LATE in its setup, after
//! restore_original_signals, and the parent's immediate `kill -INT`
//! lands while SIGINT is still SIG_DFL, so the child dies and `wait`
//! reports 130. Rubash installs the ignore in the child's pre_exec —
//! and, worse, Rust `std::process::Command::spawn()` blocks the parent
//! until the child execs (error-pipe sync), so the parent's kill can
//! never land before the ignore is in place: `wait` reports 0.
//!
//! The seed below is the issue's stable mA8 form (kill + a 0.3s probe
//! + `wait` expecting 130). `sleep 3` keeps the suite fast; the
//! mechanism is identical to the issue's `sleep 5`.
//!
//! Positive controls (both shells agree): a DELAYED kill (after
//! `sleep 0.5`) finds the ignore installed and the job survives
//! (rubash#1035 steady state); `set -m` + INT kills (130); TERM 143,
//! KILL 137, HUP 129.

use std::process::{Command, Stdio};

fn run_shell(body: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's stable seed: an immediate `kill -INT` on a fresh async
/// job must kill it; `wait` reports 130.
#[test]
fn kill_int_kills_fresh_async_job_wait_reports_130() {
    let (stdout, stderr, rc) = run_shell(
        "sleep 3 & p=$!; kill -INT $p; sleep 0.3; \
         if kill -0 $p 2>/dev/null; then echo ALIVE; else echo DEAD; fi; \
         wait $p; echo w=$?",
    );
    assert_eq!(stderr, "", "unexpected stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
    assert!(
        stdout.contains("DEAD"),
        "fresh async job survived an immediate kill -INT: {stdout:?}"
    );
    assert!(
        stdout.contains("w=130"),
        "wait did not report 130 for the INT-killed job: {stdout:?}"
    );
}

/// Delayed kill: the ignore is installed by steady state, so the job
/// survives (rubash#1035). This must stay green while the seed flips.
#[test]
fn kill_int_after_delay_leaves_async_job_alive() {
    let (stdout, _, rc) = run_shell("sleep 3 & p=$!; sleep 0.5; kill -INT $p; wait $p; echo w=$?");
    assert_eq!(rc, Some(0));
    assert!(
        stdout.contains("w=0"),
        "delayed kill -INT should leave the job alive (w=0): {stdout:?}"
    );
}

/// `set -m` + INT kills the job (130) — monitor mode never installs
/// the async ignore.
#[test]
fn kill_int_kills_monitored_async_job() {
    let (stdout, _, rc) = run_shell("set -m; sleep 3 & p=$!; kill -INT $p; wait $p; echo w=$?");
    assert_eq!(rc, Some(0));
    assert!(
        stdout.contains("w=130"),
        "monitored job should die on INT (w=130): {stdout:?}"
    );
}

/// TERM / KILL / HUP kill the job with the usual wait codes.
#[test]
fn kill_term_kill_hup_report_usual_wait_codes() {
    for (sig, want) in [("TERM", 143), ("KILL", 137), ("HUP", 129)] {
        let (stdout, _, rc) = run_shell(&format!(
            "sleep 3 & p=$!; kill -{sig} $p; wait $p; echo w=$?"
        ));
        assert_eq!(rc, Some(0));
        assert!(
            stdout.contains(&format!("w={want}")),
            "kill -{sig} should report w={want}: {stdout:?}"
        );
    }
}
