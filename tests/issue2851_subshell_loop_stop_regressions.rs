//! rubash#2851 — a foreground job stopped by SIGTSTP inside a subshell
//! in a loop must abandon the parent loop (GNU jobs.c:4341-4343
//! set_job_status_and_cleanup: `if (any_tstped && loop_level) breaking =
//! loop_level`, read by the WAITING shell across the fork). Before the
//! fix the wait inside the subshell saw loop_depth 0 (GNU
//! execute_cmd.c:1641-1642 resets the child's level) and set a count of
//! 1, which the subshell boundary consumed — so the loop continued to
//! the next iteration. The fix banks the enclosing loop depth across
//! each subshell boundary (`subshell_saved_loop_depth`, live + 1 per
//! boundary) and reads `saved + live` at the stop, so the count reaches
//! the parent loop; each boundary consumes one level on close.
//!
//! GNU boundaries, probed against the 5.3.20 oracle by this lane
//! (oracle_2851.py, job-control-parent harness):
//!   for / while / until + subshell — abandon (prompt returns, `fg`
//!     resumes the sleep, the loop never continues);
//!   nested subshells / loop-in-subshell — abandon (all levels);
//!   pipeline in a subshell in a loop — abandon;
//!   `f() ( subshell body )` in a loop — CONTINUE (function frame,
//!     loop_level 0: iter 1/2/3 all run, the stopped subshell job stays
//!     resumable via fg);
//!   plain subshell multicmd — the body abandons at the stop (INSIDE
//!     does not run until fg resumes the job).
//!
//! Harness: the shell under test runs interactive under `script(1)` (the
//! issue1016 pattern), so `^Z` (0x1a) reaches the foreground process
//! group through a real pty line discipline. Sleep is 5s so each case
//! settles in ~10s. Markers are computed values (`AFTER-*-alive`) that
//! never appear in echoed input, so a pass requires real execution.
//!
//! Known forkless-resume limitation (pre-existing, out of scope for this
//! issue): after `fg` resumes a stopped subshell, rubash does not re-run
//! the abandoned body — INSIDE never prints after fg, where GNU's forked
//! subshell resumes and prints it. These pins assert only the
//! loop-abandon/continue verdicts and the body abandon at the stop, never
//! INSIDE-after-fg.
//!
//! Eight pins: for/while/until abandonment, nested subshells, loop in a
//! subshell, pipeline in a subshell, function subshell-body continuation,
//! plain subshell body abandonment.

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
fn issue2851_for_loop_abandoned_after_stop_inside_subshell() {
    // The issue repro: `for i in 1 2 3; do (sleep 30); echo ...; done`
    // with ^Z while the subshell's sleep is foreground. GNU abandons
    // the parent loop (jobs.c:4341-4343); before the fix rubash
    // continued to iter 2 and 3.
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"for i in 1 2 3; do (sleep 5); echo ITER-$i-done; done\n",
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
fn issue2851_while_loop_abandoned_after_stop_inside_subshell() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"i=0; while [ $i -lt 3 ]; do i=$((i+1)); (sleep 5); echo WITER-$i-done; done\n",
            800,
        ),
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
fn issue2851_until_loop_abandoned_after_stop_inside_subshell() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"i=0; until [ $i -ge 3 ]; do i=$((i+1)); (sleep 5); echo UITER-$i-done; done\n",
            800,
        ),
        (b"\x1a", 800),
        (b"echo LOOPSTATUS-$?\n", 800),
        (b"jobs\n", 800),
        (b"fg\n", 6000),
        (b"echo AFTER-UNTIL-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("LOOPSTATUS-148"),
        &text,
        "abandoned until loop must report the stopped job's 148 status",
    );
    check(
        !text.contains("\r\nUITER-"),
        &text,
        "loop iterated after the stop",
    );
    check(
        text.contains("AFTER-UNTIL-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}

#[test]
fn issue2851_nested_subshells_abandoned_after_stop() {
    // Two boundaries + one loop: the count is 3 (outer boundary 1 +
    // inner boundary 1 + loop 1); each boundary consumes one, the loop
    // consumes the last. If either boundary failed to bank or consume,
    // the count leaks or undercounts and the loop continues.
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"for i in 1 2 3; do ( (sleep 5) ); echo NITER-$i-done; done\n",
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
    check(
        !text.contains("\r\nNITER-"),
        &text,
        "loop iterated after the stop inside nested subshells",
    );
    check(
        text.contains("AFTER-NESTED-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}

#[test]
fn issue2851_loop_inside_subshell_abandons_outer_loop() {
    // A loop inside a subshell inside a loop: the stop is in the inner
    // loop's body (count 3: outer boundary 1 + inner loop 1 + outer
    // loop 1). Both loops must abandon.
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"for i in 1 2 3; do (for j in a b; do sleep 5; done); echo OITER-$i-done; done\n",
            800,
        ),
        (b"\x1a", 800),
        (b"jobs\n", 800),
        (b"fg\n", 6000),
        (b"echo AFTER-OUTER-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        !text.contains("\r\nOITER-"),
        &text,
        "outer loop iterated after the stop",
    );
    check(
        text.contains("AFTER-OUTER-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}

#[test]
fn issue2851_pipeline_in_subshell_in_loop_abandoned_after_stop() {
    // The pipeline job-control wait (pipeline_exec.rs) reads the same
    // banked count as the single-child wait; the subshell boundary and
    // the loop each consume one level.
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"for i in 1 2 3; do (sleep 5 | cat); echo PITER-$i-done; done\n",
            800,
        ),
        (b"\x1a", 800),
        (b"jobs\n", 800),
        (b"fg\n", 6000),
        (b"echo AFTER-PIPE-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        !text.contains("\r\nPITER-"),
        &text,
        "loop iterated after the stop inside the pipelined subshell",
    );
    check(
        text.contains("AFTER-PIPE-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}

#[test]
fn issue2851_function_subshell_body_continues_loop_after_stop() {
    // The flat-boundary face: `f() ( ... )` runs in the function frame,
    // whose loop_level is 0 (execute_cmd.c:5359), so GNU CONTINUES the
    // loop — iter 1/2/3 all run — and the stopped subshell job stays
    // resumable. Before the fix the flat boundary never consumed the
    // slice-4 level, so it leaked into the loop driver and the loop
    // abandoned. The body itself must still abandon at the stop: the
    // iter-1 INSIDE (from the stopped call) never prints before fg;
    // INSIDE lines below come only from the iter-2/3 calls.
    // Timing: after ^Z the loop runs two more 5s sleeps (iters 2, 3);
    // the fg/echo steps queue in the tty buffer and run after the loop.
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (b"f() (sleep 5; echo INSIDE-$CHILD_MARKER)\n", 600),
        (b"for i in 1 2 3; do f; echo FITER-$i-done; done\n", 800),
        (b"\x1a", 800),
        (b"echo AFTER-FUNCLOOP-$CHILD_MARKER\n", 800),
        (b"fg\n", 12000),
        (b"echo AFTER-FG-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    // Output-line matches: the echoed input contains `echo FITER-$i-done`.
    check(
        text.contains("\r\nFITER-1-done"),
        &text,
        "loop did not continue past the stopped function subshell (iter 1 missing)",
    );
    check(
        text.contains("\r\nFITER-2-done"),
        &text,
        "loop did not reach iter 2",
    );
    check(
        text.contains("\r\nFITER-3-done"),
        &text,
        "loop did not reach iter 3",
    );
    check(
        text.contains("AFTER-FUNCLOOP-alive"),
        &text,
        "prompt did not return / marker not alive after the loop",
    );
}

#[test]
fn issue2851_plain_subshell_body_abandons_at_stop() {
    // The subshell body abandons at the stop (GNU: the forked subshell
    // stops as a unit; INSIDE runs only after fg resumes the job).
    // Plain list, no loop: nothing after the subshell is swallowed.
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (b"(sleep 5; echo INSIDE-$CHILD_MARKER)\n", 800),
        (b"\x1a", 800),
        (b"echo AFTER-STOP-$CHILD_MARKER\n", 800),
        (b"fg\n", 6000),
        (b"echo AFTER-FG2-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        !text.contains("\r\nINSIDE-"),
        &text,
        "subshell body continued after the stop",
    );
    check(
        text.contains("AFTER-STOP-alive"),
        &text,
        "prompt did not return after the stop",
    );
    check(
        text.contains("AFTER-FG2-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}
