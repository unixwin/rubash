//! Issue unixwin/rubash#1220: a `>(...)` write-side process substitution
//! consumer must not block the commands after the one carrying it.
//!
//! GNU owner: subst.c:6362 process_substitute() forks the consumer with
//! FORK_ASYNC|FORK_PROCSUB — it reads the substitution pipe concurrently
//! and the parent never waits for it (jobs.c procsub_add parks the child
//! on the procsub list, reaped by wait/SIGCHLD only). Rubash's carrier is
//! a temp file, so the body now runs as a detached `rubash -c` child fed
//! by an OS pipe (executor/external_setup.rs
//! spawn_output_process_substitution_child).
//!
//! Verified against WSL GNU Bash 5.3.20 (script-file probes): GNU prints
//! `next` in ~0s with the consumer racing in late.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Spawn rubash with the script on stdin; returns (stdout, stderr, code,
/// wall time of the rubash process itself). The reader thread drains the
/// pipes so a detached procsub child still holding them cannot stall the
/// parent's exit measurement.
fn rubash_stdin_timed(script: &str) -> (String, String, Option<i32>, Duration) {
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
    let start = Instant::now();
    let status = child.wait().expect("wait rubash");
    let elapsed = start.elapsed();
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
        elapsed,
    )
}

#[test]
fn slow_write_procsub_consumer_does_not_block() {
    // GNU: `next` prints immediately (consumer is an async child); the
    // whole rubash process must exit without waiting out the consumer.
    let (stdout, _stderr, code, elapsed) =
        rubash_stdin_timed("echo hello > >(sleep 2; echo AFTER)\necho next\n");
    assert_eq!(code, Some(0));
    assert!(
        elapsed < Duration::from_secs(2),
        "shell blocked on the consumer: {elapsed:?}"
    );
    assert!(
        stdout.contains("next"),
        "next must print: stdout={stdout:?}"
    );
}

#[test]
fn write_procsub_consumer_still_runs() {
    // The consumer still produces its output: with a following command
    // giving it time, `cat` echoes the payload once.
    let (stdout, stderr, code, _elapsed) =
        rubash_stdin_timed("echo hello > >(cat)\nsleep 0.3\necho done\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout.matches("hello").count(), 1, "stdout: {stdout}");
    assert!(stdout.contains("done"), "stdout: {stdout}");
}
