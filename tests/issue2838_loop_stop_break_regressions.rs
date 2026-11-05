//! rubash#2838 — a foreground job stopped by SIGTSTP inside a loop must
//! abandon the loop (GNU jobs.c:4335-4343 set_job_status_and_cleanup:
//! `breaking = loop_level`, "Suspending a job with SIGTSTP breaks all
//! active loops"; execute_cmd.c:3108 for, :3277 arith-for, :3840
//! while/until, :3609 select). Before the fix the wait returned the
//! Stopped job as an ordinary 148 completion, the loop started another
//! 30s sleep, and queued input was never read — the shell looked hung.
//!
//! GNU boundaries, probed against the 5.3.20 oracle by the r70 diagnosis
//! lane (pty_probe.py s13 + supplements):
//!   for / while / until — abandon (prompt returns, `fg` resumes the
//!     sleep, the loop never continues);
//!   plain list / if body / function body — continue (the next command
//!     runs immediately after the stop, no `fg` needed).
//!
//! Harness: the shell under test runs interactive under `script(1)` (the
//! issue1016 pattern), so `^Z` (0x1a) reaches the foreground process
//! group through a real pty line discipline. Sleep is 5s so each case
//! settles in ~10s. Markers are computed values (`AFTER-*-alive`) that
//! never appear in echoed input, so a pass requires real execution.
//!
//! Mutation self-check (lane-run, see REPORT.md): with the SIGTSTP
//! set-site in `wait_fg_child_job_control` disabled, the for-loop pin
//! goes red (0 passed / 1 failed in 11.63s: the loop respawns the
//! sleep, the prompt never returns, AFTER-LOOP never prints).
//!
//! Seven pins: for abandonment, while abandonment, nested-loop
//! abandonment, while-test-stop level consumption, plain-list
//! continuation, if-body continuation, function-body continuation.

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const SESSION_TIMEOUT: Duration = Duration::from_secs(40);

/// Run an interactive session under `script(1)`. Each step is
/// (bytes to send, milliseconds to settle after sending). Returns
/// None when `script(1)` is unavailable (caller skips the test).
fn run_pty_session(steps: &[(&[u8], u64)]) -> Option<String> {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("no script(1) on this host; skipping");
        return None;
    }
    let cmdline = format!("{} -i", env!("CARGO_BIN_EXE_rubash"));
    let mut child = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .env("TERM", "dumb")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn script(1)");

    let mut stdin = child.stdin.take().expect("child stdin");
    let owned_steps: Vec<(Vec<u8>, u64)> = steps
        .iter()
        .map(|(bytes, settle)| (bytes.to_vec(), *settle))
        .collect();
    let writer = std::thread::spawn(move || {
        for (bytes, settle) in owned_steps {
            if stdin.write_all(&bytes).is_err() {
                break;
            }
            let _ = stdin.flush();
            std::thread::sleep(Duration::from_millis(settle));
        }
    });

    let mut stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    let buf = match rx.recv_timeout(SESSION_TIMEOUT) {
        Ok(buf) => buf,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            let buf = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
            let _ = reader.join();
            let text = String::from_utf8_lossy(&buf).into_owned();
            panic!("interactive session did not finish within {SESSION_TIMEOUT:?} (prompt never returned or shell hung); transcript: {text:?}");
        }
    };
    let status = child.wait().expect("wait for script(1)");
    let _ = writer.join();
    let _ = reader.join();
    assert_eq!(
        status.code(),
        Some(0),
        "transcript: {:?}",
        String::from_utf8_lossy(&buf)
    );
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn check(condition: bool, text: &str, what: &str) {
    assert!(condition, "{what}; transcript: {text:?}");
}

#[test]
fn issue_2838_for_loop_abandoned_after_sigtstp_stop() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"for i in 1 2 3; do sleep 5; echo ITER-$i-done; done\n",
            800,
        ),
        (b"\x1a", 800),
        (b"echo LOOPSTATUS-$?\n", 800),
        (b"jobs\n", 800),
        (b"fg\n", 6000),
        (b"echo AFTER-LOOP-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("LOOPSTATUS-148"),
        &text,
        "abandoned for loop must report the stopped job's 148 status",
    );
    // Output-line match ("\r\nITER-..."): the echoed input line itself
    // contains `echo ITER-$i-done`, so a bare substring check would
    // match the echo, not real output.
    check(
        !text.contains("\r\nITER-"),
        &text,
        "loop iterated after the stop",
    );
    check(
        text.contains("AFTER-LOOP-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}

#[test]
fn issue_2838_while_loop_abandoned_after_sigtstp_stop() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (b"while true; do sleep 5; echo WITER-done; done\n", 800),
        (b"\x1a", 800),
        (b"echo LOOPSTATUS-$?\n", 800),
        (b"jobs\n", 800),
        (b"fg\n", 6000),
        (b"echo AFTER-WHILE-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("LOOPSTATUS-148"),
        &text,
        "abandoned while loop must report the stopped job's 148 status",
    );
    // Output-line match: the echoed input line contains `echo WITER-done`.
    check(
        !text.contains("\r\nWITER-"),
        &text,
        "loop iterated after the stop",
    );
    check(
        text.contains("AFTER-WHILE-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}

#[test]
fn issue_2838_nested_loops_all_abandoned_after_sigtstp_stop() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"for i in 1 2; do for j in 1 2; do sleep 5; echo INNER-$i-$j; done; echo OUTER-$i; done\n",
            800,
        ),
        (b"\x1a", 800),
        (b"jobs\n", 800),
        (b"fg\n", 6000),
        (b"echo AFTER-NESTED-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    // Output-line matches: the echoed input line contains
    // `echo INNER-$i-$j` / `echo OUTER-$i`.
    check(
        !text.contains("\r\nINNER-"),
        &text,
        "inner loop iterated after the stop",
    );
    check(
        !text.contains("\r\nOUTER-"),
        &text,
        "outer loop iterated after the stop",
    );
    check(
        text.contains("AFTER-NESTED-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}

#[test]
fn issue_2838_while_test_stop_consumes_one_breaking_level() {
    // The stop lands in the inner while's TEST, inside an outer for:
    // breaking is set to 2. The while test-side exit must consume one
    // level (GNU execute_cmd.c:3819-3821); the outer for's post-body
    // check consumes the other, so the prompt returns clean and the
    // next top-level command runs. If the test-side decrement were
    // missing, the leaked count would make the list-walker guard
    // (execute_cmd.c:635) swallow AFTER-TESTSTOP as well.
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"for i in 1 2; do while sleep 5; do echo INNER-W; done; echo OUTER-AFTER-$i; done\n",
            800,
        ),
        (b"\x1a", 800),
        (b"jobs\n", 800),
        (b"fg\n", 6000),
        (b"echo AFTER-TESTSTOP-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        !text.contains("\r\nINNER-W"),
        &text,
        "while body ran after the test stop",
    );
    check(
        !text.contains("\r\nOUTER-AFTER-"),
        &text,
        "outer loop continued after the stop",
    );
    check(
        text.contains("AFTER-TESTSTOP-alive"),
        &text,
        "breaking count leaked past the loops and swallowed the next command",
    );
}

#[test]
fn issue_2838_plain_list_continues_after_sigtstp_stop() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (b"sleep 5; echo AFTER-LIST\n", 800),
        (b"\x1a", 800),
        (b"echo MARK-LIST-$CHILD_MARKER\n", 800),
        (b"kill -CONT %1\n", 6000), // resume the stopped sleep so it finishes and is reaped; a bare `kill -KILL %1` + immediate `exit` races the async death notice (jobs-table family, out of scope here)
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("AFTER-LIST"),
        &text,
        "plain list must continue to the next command after the stop, without fg",
    );
    check(
        text.contains("MARK-LIST-alive"),
        &text,
        "prompt did not return after the list finished",
    );
}

#[test]
fn issue_2838_if_body_continues_after_sigtstp_stop() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (b"if true; then sleep 5; echo AFTER-IF; fi\n", 800),
        (b"\x1a", 800),
        (b"echo MARK-IF-$CHILD_MARKER\n", 800),
        (b"kill -CONT %1\n", 6000), // resume the stopped sleep so it finishes and is reaped; a bare `kill -KILL %1` + immediate `exit` races the async death notice (jobs-table family, out of scope here)
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("AFTER-IF"),
        &text,
        "if body must continue to the next command after the stop, without fg",
    );
    check(
        text.contains("MARK-IF-alive"),
        &text,
        "prompt did not return after the if finished",
    );
}

#[test]
fn issue_2838_function_body_continues_after_sigtstp_stop() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (b"f() { sleep 5; echo AFTER-FUNC; }\n", 600),
        (b"f\n", 800),
        (b"\x1a", 800),
        (b"echo MARK-FUNC-$CHILD_MARKER\n", 800),
        (b"kill -CONT %1\n", 6000), // resume the stopped sleep so it finishes and is reaped; a bare `kill -KILL %1` + immediate `exit` races the async death notice (jobs-table family, out of scope here)
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("AFTER-FUNC"),
        &text,
        "function body must continue to the next command after the stop, without fg (GNU resets loop_level inside functions, execute_cmd.c:5359)",
    );
    check(
        text.contains("MARK-FUNC-alive"),
        &text,
        "prompt did not return after the function finished",
    );
}
