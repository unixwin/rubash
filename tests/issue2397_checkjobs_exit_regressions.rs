//! rubash#2397 — interactive `exit` lacked GNU's job protection:
//! with running jobs the first exit went through immediately, and
//! stopped jobs never protected at all.
//!
//! GNU contract (bash 5.3 C source): builtins/exit.def
//! `exit_or_logout` — `exit_immediate_okay` is true only when the
//! shell is non-interactive or the previously dispatched builtin
//! (builtins/common.c:69-70 this/last_shell_builtin, rolled at
//! execute_cmd.c:4810-4813) was exit, logout or jobs. Otherwise the
//! job table is scanned: any STOPPED job defers with "There are
//! stopped jobs." (NOT gated on checkjobs); else, with
//! check_jobs_at_exit (shell.c:153, the checkjobs shopt), any
//! RUNNING job defers with "There are running jobs.". The listing
//! (list_all_jobs, JLIST_STANDARD) goes to stdout only when
//! checkjobs is set. A deferred exit prints the message, pins
//! this = last = exit_builtin (so the NEXT exit is immediate),
//! returns EXECUTION_FAILURE (status 1), never parses its argument
//! and runs no exit trap. parse.y handle_eof_input_unit runs the
//! same exit_builtin for an interactive EOF, so EOF defers too
//! (without touching the last exit status).
//!
//! Status mechanics, pinned down against the oracle (GNU bash
//! 5.3.20, driven over pipes exactly like this harness): a deferred
//! TYPED exit sets $? = 1 — read it immediately with an echo and it
//! prints 1. The session's FINAL exit code is whatever $? holds
//! when the immediate exit fires: an intervening successful command
//! overwrites the deferral's 1 with its own 0, and an EOF deferral
//! leaves $? alone (handle_eof_input_unit, parse.y:6973-6992,
//! discards exit_builtin's return). So the seed shape below ends
//! with code 0 on GNU, not 1 — an earlier draft of these pins
//! asserted 1 for the final code and was wrong; the deferral's own
//! status is asserted where it is observable, in DEFER_STATUS.
//!
//! Harness: `rubash -i` over pipes (same shape as
//! tests/issue2214_job_start_notice_regressions.rs), with the
//! session's final status captured. The running-job expectations
//! were verified byte-for-byte against GNU bash 5.3.20 driven the
//! same way. Harness caveat for the STOPPED forms: over pipes GNU
//! has no job control, so its waitchld never learns of the stop and
//! the job still reads Running there; the stopped verdicts are
//! GNU's job-control (PTY) semantics, which is the environment the
//! issue's seed describes. rubash's job table learns of the
//! shell-sent stop at its reap-observation points; the stopped
//! pins insert a foreground `sleep 1` as that observation point
//! (an external command — it never touches the builtin tracker),
//! after which the exit under test observes Stopped exactly as
//! GNU does on a terminal.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2397-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create sandbox dir");
    fs::canonicalize(&dir).expect("canonicalize sandbox")
}

/// Run one interactive pipe session feeding exactly `lines`, then
/// EOF; return (stdout, stderr, exit status).
fn run_interactive(home: &PathBuf, lines: &[&str]) -> (String, String, i32) {
    let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    let mut stdin = child.stdin.take().expect("child stdin");
    let mut script = lines.join("\n");
    script.push('\n');
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(script.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    });
    let (tx, rx) = mpsc::channel();
    let mut out = child.stdout.take().expect("child stdout");
    let t_out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let (tx2, rx2) = mpsc::channel();
    let mut err = child.stderr.take().expect("child stderr");
    let t_err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        let _ = tx2.send(buf);
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait failed: {e}"),
        }
    };
    let _ = writer.join();
    let stdout = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let stderr = rx2.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = t_out.join();
    let _ = t_err.join();
    let code = status.and_then(|s| s.code()).expect("session exit status");
    (
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
        code,
    )
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

#[test]
fn seed_running_job_defers_exit_with_checkjobs() {
    // The issue's seed (sleep lengthened 2 -> 30 so the job cannot
    // finish mid-session on a loaded machine).
    let home = sandbox("seed");
    let (stdout, stderr, code) = run_interactive(
        &home,
        &[
            "set -m",
            "shopt -s checkjobs",
            "sleep 30 >/dev/null 2>&1 &",
            "exit",
            "echo DEFER_STATUS=$?",
            "echo still",
        ],
    );
    assert!(
        stderr.contains("There are running jobs."),
        "first exit must defer:\nstderr={stderr}"
    );
    assert!(
        stdout.contains("DEFER_STATUS=1"),
        "the deferred exit's own status is EXECUTION_FAILURE (1), \
         read before anything overwrites it:\nstdout={stdout}"
    );
    assert!(
        stdout.contains("still"),
        "the line after the deferred exit must run:\nstdout={stdout}"
    );
    assert!(
        stdout.contains("Running"),
        "the deferral lists the jobs on stdout:\nstdout={stdout}"
    );
    // The trailing EOF defers once more (the echoes rolled the
    // tracker), then the next EOF exits. The final code is 0, NOT
    // the deferral's 1: `echo still` succeeded after the deferral,
    // and the EOF deferral does not touch the status, so the
    // immediate EOF exit leaves with the echo's 0. GNU bash 5.3.20
    // driven the same way ends this exact session with 0 too.
    assert_eq!(
        count(&stderr, "There are running jobs."),
        2,
        "typed exit + EOF each defer once:\nstderr={stderr}"
    );
    assert_eq!(code, 0, "final status is the intervening echo's, per GNU");
}

#[test]
fn consecutive_exits_second_is_immediate() {
    let home = sandbox("consecutive");
    let (stdout, stderr, code) = run_interactive(
        &home,
        &[
            "set -m",
            "shopt -s checkjobs",
            "sleep 30 >/dev/null 2>&1 &",
            "exit",
            "exit",
            "echo still",
        ],
    );
    assert_eq!(
        count(&stderr, "There are running jobs."),
        1,
        "only the first exit defers:\nstderr={stderr}"
    );
    assert!(
        !stdout.contains("still"),
        "the second exit exits, so `still` never runs:\nstdout={stdout}"
    );
    assert_eq!(code, 1);
}

#[test]
fn jobs_then_exit_is_immediate() {
    // exit_immediate_okay includes last_shell_builtin == jobs.
    let home = sandbox("jobsfirst");
    let (stdout, stderr, code) = run_interactive(
        &home,
        &[
            "set -m",
            "shopt -s checkjobs",
            "sleep 30 >/dev/null 2>&1 &",
            "jobs",
            "exit",
            "echo still",
        ],
    );
    assert!(
        !stderr.contains("There are"),
        "no deferral after `jobs`:\nstderr={stderr}"
    );
    assert!(!stdout.contains("still"), "stdout={stdout}");
    assert_eq!(code, 0);
}

#[test]
fn checkjobs_off_running_job_exits_immediately() {
    // Control (GNU SAME): running jobs do not protect without the
    // shopt.
    let home = sandbox("offrunning");
    let (stdout, stderr, code) = run_interactive(
        &home,
        &["set -m", "sleep 30 >/dev/null 2>&1 &", "exit", "echo still"],
    );
    assert!(!stderr.contains("There are"), "stderr={stderr}");
    assert!(!stdout.contains("still"), "stdout={stdout}");
    assert_eq!(code, 0);
}

#[test]
fn eof_defers_once_with_running_job() {
    let home = sandbox("eof");
    let (stdout, stderr, code) = run_interactive(
        &home,
        &["set -m", "shopt -s checkjobs", "sleep 30 >/dev/null 2>&1 &"],
    );
    assert_eq!(
        count(&stderr, "There are running jobs."),
        1,
        "the first EOF defers, the second exits:\nstderr={stderr}"
    );
    assert!(
        stdout.contains("Running"),
        "the EOF deferral lists the jobs:\nstdout={stdout}"
    );
    assert_eq!(code, 0, "an EOF deferral leaves the last status alone");
}

#[test]
fn deferred_exit_never_parses_its_argument() {
    let home = sandbox("arg");
    let (stdout, stderr, code) = run_interactive(
        &home,
        &[
            "set -m",
            "shopt -s checkjobs",
            "sleep 30 >/dev/null 2>&1 &",
            "exit 5",
            "echo DEFER_STATUS=$?",
            "echo still",
        ],
    );
    assert!(
        stderr.contains("There are running jobs."),
        "stderr={stderr}"
    );
    assert!(
        stdout.contains("DEFER_STATUS=1"),
        "the deferral's status is 1, not the argument 5 — the \
         argument is never parsed on the deferral path:\nstdout={stdout}"
    );
    assert!(
        stdout.contains("still"),
        "`exit 5` deferred, so the shell continues:\nstdout={stdout}"
    );
    // Final code is the intervening echo's 0, exactly as GNU ends
    // this session (see the header note on status mechanics).
    assert_eq!(code, 0, "final status is the intervening echo's, per GNU");
}

#[test]
fn stopped_job_defers_with_checkjobs_on() {
    let home = sandbox("stoppedon");
    let (stdout, stderr, _code) = run_interactive(
        &home,
        &[
            "set -m",
            "shopt -s checkjobs",
            "sleep 30 >/dev/null 2>&1 &",
            "kill -STOP %1",
            "sleep 1",
            "exit",
            "echo still",
        ],
    );
    assert!(
        stderr.contains("There are stopped jobs."),
        "stderr={stderr}"
    );
    assert!(stdout.contains("still"), "stdout={stdout}");
    assert!(
        stdout.contains("Stopped"),
        "checkjobs on: the deferral lists the stopped job:\nstdout={stdout}"
    );
}

#[test]
fn stopped_job_defers_with_checkjobs_off() {
    // The stopped protection is independent of checkjobs; only the
    // listing is gated on the shopt.
    let home = sandbox("stoppedoff");
    let (stdout, stderr, _code) = run_interactive(
        &home,
        &[
            "set -m",
            "sleep 30 >/dev/null 2>&1 &",
            "kill -STOP %1",
            "sleep 1",
            "exit",
            "echo still",
        ],
    );
    assert!(
        stderr.contains("There are stopped jobs."),
        "stderr={stderr}"
    );
    assert!(stdout.contains("still"), "stdout={stdout}");
    assert!(
        !stdout.contains("Stopped"),
        "checkjobs off: no job listing on stdout:\nstdout={stdout}"
    );
}
