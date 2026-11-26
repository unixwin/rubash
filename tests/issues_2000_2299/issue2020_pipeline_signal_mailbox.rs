//! rubash#2020 regression pins (Windows face): the engine's pipeline
//! blocking waits never polled the signal mailbox, so under an event-shaped
//! Ctrl+C a pure external-command pipeline only settled when its stage ran
//! to natural end — the niu host handler recorded the SIGINT in the engine
//! mailbox (kill.rs note_console_ctrl_c -> queue_self_signal) but nothing
//! on the pipeline wait path consumed it.
//!
//! GNU model: jobs.c:3064 wait_for installs wait_sigint_handler
//! (jobs.c:3090-3101, handler at jobs.c:2908) for the foreground wait, so a
//! ^C recorded by trap.c:545 trap_handler is observed while the shell is
//! blocked and the job settles 128+SIGINT = 130 at once; a TRAPPED SIGINT
//! is only recorded (trap.c:537-543 pending_traps[sig]++) and its action
//! defers to the command boundary (execute_cmd.c:643 run_pending_traps —
//! bash manual, SIGNALS: "the trap will not be executed until the command
//! completes").
//!
//! The injection here is the production cross-process face (`kill -INT
//! <pid>` from a second rubash -> deliver_rubash_signal mailbox entry ->
//! observe_signals_while_blocked), the same queue an event-shaped console
//! ^C lands in. Expected outputs per the issue body's GNU bash 5.3.20
//! contract: ^C settles `ping -n 30 127.0.0.1 | cat` immediately with
//! status 130; a trapped ^C does not kill the pipeline and fires after it
//! completes.

#![cfg(windows)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Hard ceiling for an interrupted probe. The natural end of `ping -n 30`
/// is ~30s, so anything past this ceiling means the interrupt was slept
/// through (the defect this issue pins).
const INTERRUPT_LIMIT: Duration = Duration::from_secs(15);

#[derive(Debug)]
struct RunOutcome {
    timed_out: bool,
    elapsed: Duration,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Run `script` under rubash; once the script prints `started` (after its
/// own mailbox-registration settle sleep), inject an untrapped SIGINT from
/// a second rubash process — the production cross-process mailbox face.
fn run_pipeline_probe(tag: &str, script: &str, inject_at_started: bool) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue2020-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue2020 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .spawn()
        .expect("spawn rubash victim");

    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let mut stderr_pipe = child.stderr.take().expect("piped stderr");
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr_pipe, &mut buffer);
        buffer
    });

    let start = Instant::now();
    let mut seen = String::new();
    if inject_at_started {
        // Wait for the pipeline to be underway: the script prints `started`
        // after a settle sleep (the async mailbox registration in
        // Executor::new is long since done), then spawns the pipeline.
        let mut chunk = [0u8; 1024];
        let marker_deadline = Instant::now() + Duration::from_secs(10);
        while !seen.contains("started") && Instant::now() < marker_deadline {
            match stdout_pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => seen.push_str(&String::from_utf8_lossy(&chunk[..n])),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        assert!(
            seen.contains("started"),
            "[{tag}] pipeline never started: {seen}"
        );
        // Give the pipeline members a beat to spawn, then ^C through the
        // mailbox (a second rubash: deliver_rubash_signal's production face).
        std::thread::sleep(Duration::from_millis(400));
        let killer_pid = child.id();
        let killer = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .args(["-c", &format!("kill -INT {killer_pid}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env_remove("BASH_ENV")
            .status()
            .expect("spawn rubash killer");
        assert!(killer.success(), "[{tag}] kill -INT {killer_pid} failed");
    }

    let (timed_out, code) = loop {
        match child.try_wait().expect("poll rubash victim") {
            Some(status) => break (false, status.code()),
            None => {
                if start.elapsed() > INTERRUPT_LIMIT {
                    let _ = child.kill();
                    let _ = child.wait();
                    break (true, None);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    };
    let elapsed = start.elapsed();
    let mut rest = Vec::new();
    let _ = std::io::Read::read_to_end(&mut stdout_pipe, &mut rest);
    let mut stdout = seen;
    stdout.push_str(&String::from_utf8_lossy(&rest));
    let stderr =
        String::from_utf8_lossy(&stderr_reader.join().expect("stderr reader")).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        timed_out,
        elapsed,
        code,
        stdout,
        stderr,
    }
}

/// The issue body's acceptance probe: an event-shaped ^C during a pure
/// external pipeline (`ping -n 30 127.0.0.1 | cat`) settles immediately
/// with exit status 130 instead of waiting out the producer's natural end.
#[test]
fn issue2020_event_int_settles_pure_external_pipeline() {
    let script = "sleep 0.5\necho started\nping -n 30 127.0.0.1 | cat\necho settled=$?\n";
    let outcome = run_pipeline_probe("pure-external", script, true);
    assert!(
        !outcome.timed_out,
        "pipeline slept through the ^C (ran past {INTERRUPT_LIMIT:?}): {outcome:?}"
    );
    assert_eq!(
        outcome.code,
        Some(130),
        "exit status: {:?} stdout={} stderr={}",
        outcome.code,
        outcome.stdout,
        outcome.stderr
    );
    assert!(
        !outcome.stdout.contains("settled="),
        "script continued past the aborted pipeline: {}",
        outcome.stdout
    );
}

/// The last-member face: the producer exits instantly and the shell blocks
/// waiting the EXTERNAL last member (`wait_pipeline_member_interruptibly`).
/// The recorded ^C must end that wait with 130 too.
#[test]
fn issue2020_event_int_settles_last_external_member_wait() {
    let script = "sleep 0.5\necho started\necho hi | ping -n 30 127.0.0.1\necho settled=$?\n";
    let outcome = run_pipeline_probe("last-member", script, true);
    assert!(
        !outcome.timed_out,
        "last-member wait slept through the ^C: {outcome:?}"
    );
    assert_eq!(outcome.code, Some(130), "stdout={}", outcome.stdout);
    assert!(!outcome.stdout.contains("settled="));
}

/// GNU's deferral half (trap.c:545 records, execute_cmd.c:643 dispatches at
/// the boundary): a TRAPPED SIGINT must not kill the pipeline early — the
/// stage runs to natural end, then the trap action fires.
#[test]
fn issue2020_trapped_int_defers_until_pipeline_completes() {
    let script = "sleep 0.5\necho started\ntrap 'echo TRAPPED; exit 7' INT\nping -n 4 127.0.0.1 | cat\necho after=$?\n";
    let outcome = run_pipeline_probe("trapped-defers", script, true);
    assert!(
        !outcome.timed_out,
        "trapped probe overran its natural end: {outcome:?}"
    );
    // The pipeline ran to natural end (~4s): the trap did not kill it early.
    assert!(
        outcome.elapsed >= Duration::from_secs(3),
        "trapped ^C killed the pipeline early ({}ms): {}",
        outcome.elapsed.as_millis(),
        outcome.stdout
    );
    assert!(
        outcome.stdout.contains("TRAPPED"),
        "trap action never ran: {}",
        outcome.stdout
    );
    assert_eq!(outcome.code, Some(7), "stdout={}", outcome.stdout);
}

/// No-signal sanity: the drain+wait restructure keeps the plain pipeline
/// semantics byte-identical (payload intact, status 0, prompt exit).
#[test]
fn issue2020_plain_pipeline_still_settles_normally() {
    let script = "ping -n 2 127.0.0.1 | cat\necho settled=$?\n";
    let outcome = run_pipeline_probe("plain", script, false);
    assert!(!outcome.timed_out, "plain pipeline overran: {outcome:?}");
    assert_eq!(outcome.code, Some(0), "stdout={}", outcome.stdout);
    assert!(outcome.stdout.contains("settled=0"), "{}", outcome.stdout);
    assert!(
        outcome.stdout.contains("TTL") || outcome.stdout.contains("Reply"),
        "ping payload lost through the drain: {}",
        outcome.stdout
    );
}

/// The merged-stream face (`2>&1 |` / `|&` — ONE pipe drained while the
/// stage runs) keeps its payload and settles 0 without a signal.
#[test]
fn issue2020_merged_stream_pipeline_still_settles_normally() {
    let script = "ping -n 2 127.0.0.1 2>&1 | cat\necho settled=$?\n";
    let outcome = run_pipeline_probe("merged", script, false);
    assert!(!outcome.timed_out, "merged pipeline overran: {outcome:?}");
    assert_eq!(outcome.code, Some(0), "stdout={}", outcome.stdout);
    assert!(outcome.stdout.contains("settled=0"), "{}", outcome.stdout);
    assert!(
        outcome.stdout.contains("TTL") || outcome.stdout.contains("Reply"),
        "ping payload lost through the merged drain: {}",
        outcome.stdout
    );
}
