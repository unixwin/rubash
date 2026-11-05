//! rubash#2836 slice 4 — pipeline stages and external children inside
//! an in-process subshell get job-control treatment (their own process
//! group + terminal handover + WUNTRACED wait) when job control is
//! active, and a stopped child is parked in the job table per the #2838
//! stop-unwind mechanism.
//!
//! GNU boundaries, probed against the 5.3.20 oracle by this lane
//! (~/workspace/gap12-probes/oracle_s4{,b,c}.py + oracle_s4d.py):
//!   `sleep 30 | cat` + ^Z — ONE job `[1]+ Stopped sleep 30 | cat`;
//!     `$?` = 148; PIPESTATUS NOT updated (stale); same-line `; echo`
//!     runs at once; `fg` resumes the whole pipeline; `kill -KILL %1`
//!     killpgs both stages (`jobs` shows `Killed`); `jobs -l` lists
//!     both pids with stage texts.
//!   `(sleep 30)` + ^Z — `[1]+ Stopped ( sleep 30 )`; `$?` = 148; `fg`
//!     resumes the subshell; `kill -KILL %1` works.
//!   `(sleep 2 | cat)` + ^Z — ONE job `( sleep 2 | cat )` (parens);
//!     `jobs -l` shows ONE pid (the subshell's); `fg` resumes it.
//!   A pipeline/subshell stop inside `for` ABANDONS the loop.
//!   `(sleep 2; echo INSIDE)` + ^Z — INSIDE does NOT run at the stop;
//!     GNU runs INSIDE after `fg` (whole subshell resumes). Rubash
//!     cannot resume the frame (in-process model) — brief-accepted
//!     divergence: fg resumes only the sleep child; INSIDE never runs.
//!
//! Harness: the shell under test runs interactive under `script(1)` (the
//! issue1016 pattern), so `^Z` (0x1a) reaches the foreground process
//! group through a real pty line discipline. Sleep is 2s (5s for the
//! kill cases, for timing margin) so each case settles in ~15s. `$?`
//! and PIPESTATUS are captured atomically in one assignment
//! (`SAVED="RC=$? PS=${PIPESTATUS[*]}"`) because any intervening
//! command — including the same-line echo — clobbers both. Markers are
//! computed values (`AFTER-*-alive`) that never appear in echoed input,
//! so a pass requires real execution.
//!
//! Mutation self-check (lane-run, see REPORT.md): (a) with the
//! stop-signal check in `wait_pipeline_job_control` flipped to
//! SIGTTIN, the pipeline pin goes red (no Stopped notice, prompt never
//! returns); (b) with the subshell counter arm in
//! `wait_fg_child_job_control` removed, the multi-command pin goes red
//! (INSIDE prints at the stop).
//!
//! Seven pins: pipeline stop + status + stale PIPESTATUS + same-line +
//! fg + jobs-empty; pipeline loop abandonment; pipeline kill %1;
//! subshell stop + status + same-line + fg; subshell multi-command
//! abandonment; subshell kill %1; pipeline-in-subshell stop + fg.

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

/// Text between the last occurrence of `start` and the first occurrence
/// of `end` after it. Both markers are echoed input lines, so the
/// segment is the commands' output between them.
fn segment<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let Some(after_start) = text.rfind(start).map(|i| i + start.len()) else {
        return "";
    };
    let rest = &text[after_start..];
    let Some(end_at) = rest.find(end) else {
        return rest;
    };
    &rest[..end_at]
}

#[test]
fn issue_2836_pipeline_stop_status_pipestatus_stale_sameline_fg() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (b"false | true\n", 600),
        (
            b"sleep 2 | cat; SAVED=\"RC=$? PS=${PIPESTATUS[*]}\"; echo AFTER-PIPE\n",
            800,
        ),
        (b"\x1a", 800),
        (b"echo SAVED-$SAVED\n", 800),
        (b"jobs\n", 800),
        (b"fg\n", 4000),
        (b"echo FGSTATUS-$?\n", 800),
        (b"echo BEFORE-JOBS2\n", 600),
        (b"jobs\n", 800),
        (b"echo AFTER-JOBS2\n", 800),
        (b"echo AFTER-FG-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    // Atomic capture: the same-line echo would clobber both if read
    // separately. `false | true` set PIPESTATUS to `1 0`; the stopped
    // pipeline must leave it untouched.
    check(
        text.contains("SAVED-RC=148 PS=1 0"),
        &text,
        "stopped pipeline must report 148 and leave PIPESTATUS stale at `1 0`",
    );
    check(
        text.contains("AFTER-PIPE"),
        &text,
        "same-line command must run at once after the stop",
    );
    // The joined text appears in the echoed input once; the stop notice
    // and the jobs listing add two more. A bare count >= 2 proves the
    // text reached real output.
    check(
        text.matches("sleep 2 | cat").count() >= 2,
        &text,
        "pipeline job text `sleep 2 | cat` missing from notice/jobs output",
    );
    check(
        text.contains("FGSTATUS-0"),
        &text,
        "fg must resume the whole pipeline to a 0 status",
    );
    check(
        !segment(&text, "BEFORE-JOBS2", "AFTER-JOBS2").contains("[1]"),
        &text,
        "fg must consume the pipeline job (jobs must be empty after fg)",
    );
    check(
        text.contains("AFTER-FG-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}

#[test]
fn issue_2836_pipeline_loop_abandoned_after_sigtstp_stop() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"for i in 1 2 3; do sleep 2 | cat; echo ITER-$i-done; done\n",
            800,
        ),
        (b"\x1a", 800),
        (b"echo LOOPSTATUS-$?\n", 800),
        (b"jobs\n", 800),
        (b"fg\n", 4000),
        (b"echo AFTER-LOOP-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("LOOPSTATUS-148"),
        &text,
        "abandoned pipeline loop must report the stopped job's 148 status",
    );
    // Output-line match: the echoed input line contains `echo ITER-$i-done`.
    check(
        !text.contains("\r\nITER-"),
        &text,
        "loop iterated after the pipeline stop",
    );
    check(
        text.contains("AFTER-LOOP-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}

#[test]
fn issue_2836_pipeline_kill_percent_kills_both_stages() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (b"sleep 5 | cat\n", 800),
        (b"\x1a", 800),
        (b"kill -KILL %1\n", 1500),
        (b"jobs\n", 800),
        (b"echo AFTER-KILL-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("Killed"),
        &text,
        "kill -KILL %1 must killpg the whole pipeline job",
    );
    check(
        text.contains("AFTER-KILL-alive"),
        &text,
        "prompt did not return / marker not alive after the kill",
    );
}

#[test]
fn issue_2836_subshell_stop_status_and_sameline_fg() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (b"(sleep 2); SAVED=\"RC=$?\"; echo AFTER-SUB\n", 800),
        (b"\x1a", 800),
        (b"echo SAVED-$SAVED\n", 800),
        (b"jobs\n", 800),
        (b"fg\n", 4000),
        (b"echo AFTER-SUBFG-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("SAVED-RC=148"),
        &text,
        "stopped subshell must report the 148 status",
    );
    check(
        text.contains("AFTER-SUB"),
        &text,
        "same-line command must run at once after the stop",
    );
    // The notice prints the GNU shape `( sleep 2 )` (with spaces); the
    // echoed input has `(sleep 2)` (without), so a bare substring
    // proves the text reached real output.
    check(
        text.contains("( sleep 2 )"),
        &text,
        "subshell job text `( sleep 2 )` missing from notice/jobs output",
    );
    check(
        text.contains("AFTER-SUBFG-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}

#[test]
fn issue_2836_subshell_multi_command_body_abandoned() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"(sleep 2; echo INSIDE); SAVED=\"RC=$?\"; echo AFTER-MULTI\n",
            800,
        ),
        (b"\x1a", 800),
        (b"echo SAVED-$SAVED\n", 800),
        (b"fg\n", 4000),
        (b"echo AFTER-MULTIFG-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("SAVED-RC=148"),
        &text,
        "stopped multi-command subshell must report the 148 status",
    );
    check(
        text.contains("AFTER-MULTI"),
        &text,
        "same-line command must run at once after the stop",
    );
    // Output-line match: the echoed input line contains `echo INSIDE`.
    // INSIDE must never run — neither at the stop nor after fg (the
    // in-process subshell frame cannot resume; brief-accepted R2).
    check(
        !text.contains("\r\nINSIDE"),
        &text,
        "subshell body continued after the stop (INSIDE ran)",
    );
    check(
        text.contains("AFTER-MULTIFG-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}

#[test]
fn issue_2836_subshell_kill_percent_kills_child() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (b"(sleep 5)\n", 800),
        (b"\x1a", 800),
        (b"kill -KILL %1\n", 1500),
        (b"jobs\n", 800),
        (b"echo AFTER-SUBKILL-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("Killed"),
        &text,
        "kill -KILL %1 must kill the stopped subshell child",
    );
    check(
        text.contains("AFTER-SUBKILL-alive"),
        &text,
        "prompt did not return / marker not alive after the kill",
    );
}

#[test]
fn issue_2836_pipeline_in_subshell_stop_and_fg() {
    let Some(text) = run_pty_session(&[
        (b"CHILD_MARKER=alive\n", 600),
        (
            b"(sleep 2 | cat); SAVED=\"RC=$?\"; echo AFTER-SUBPIPE\n",
            800,
        ),
        (b"\x1a", 800),
        (b"echo SAVED-$SAVED\n", 800),
        (b"jobs\n", 800),
        (b"fg\n", 4000),
        (b"echo AFTER-SUBPIPEFG-$CHILD_MARKER\n", 800),
        (b"exit\n", 500),
    ]) else {
        return;
    };
    check(text.contains("Stopped"), &text, "stopped notice missing");
    check(
        text.contains("SAVED-RC=148"),
        &text,
        "stopped pipeline-in-subshell must report the 148 status",
    );
    check(
        text.contains("AFTER-SUBPIPE"),
        &text,
        "same-line command must run at once after the stop",
    );
    // GNU's job for a `( ... )` is the subshell pipeline itself:
    // `( sleep 2 | cat )` with spaces (oracle_s4d.py). The echoed
    // input has `(sleep 2 | cat)` without, so a bare substring proves
    // the text reached real output.
    check(
        text.contains("( sleep 2 | cat )"),
        &text,
        "pipeline-in-subshell job text `( sleep 2 | cat )` missing from notice/jobs output",
    );
    check(
        text.contains("AFTER-SUBPIPEFG-alive"),
        &text,
        "prompt did not return / marker not alive after fg",
    );
}
