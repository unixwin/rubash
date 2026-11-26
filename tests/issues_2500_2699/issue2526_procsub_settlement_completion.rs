//! rubash#2526 — procsub boundary/job-table settlement, asserted in
//! COMPLETION FORM (output bytes, exit status, and file existence at
//! shell exit), never with absolute time thresholds.
//!
//! Facecheck history: the headline 10.3s block was adjudicated on
//! 2026-11-02 as a harness artifact (the pin measured its own pipe
//! drain; see tests/issue1959_procsub_argv_streaming.rs) and the
//! bare-`wait` membership cells were locked with wide timing bounds
//! in tests/issue2526_procsub_wait_settlement.rs (all matching GNU on
//! master 88047f50). This file re-locks the same three membership
//! cells plus the abandoned-producer boundary with assertions that
//! cannot flip under load: a producer that sleeps N seconds and then
//! touches a marker file has either been waited out (marker present
//! at shell exit) or not (marker absent) — the answer is a fact about
//! the run, not a race against a clock.
//!
//! GNU semantics: jobs.c:2796 wait_for_background_pids settles a
//! procsub child only when it is also the last asynchronous pid
//! (process_substitute registers its child there, subst.c:6479
//! procsub_add); a later real background job overwrites the
//! registration and the producer is out of the wait set. A consumer
//! of an output procsub is the last registration in its shape, so
//! bare `wait` drains it to completion.
//!
//! Oracle (GNU bash 5.3.20, script-file probe on this lane):
//!   shape 1: stdout `first\nsdone1\n`, marker1 absent at exit
//!   shape 2: result.txt == `x\n` at exit, stdout `waited\n`
//!   shape 3: stdout `x:ab\ndone2\n`, marker2 PRESENT at exit
//!   shape 4: stdout `y:cd\ndone3\n`, marker3 absent at exit
//! The base tree's borrowed-binary probe reproduced all four shapes
//! byte-for-byte, so these are confirmation pins: they fail only if
//! a future change breaks settlement membership in either direction
//! (waiting producers that must be abandoned, or dropping consumers
//! that must be settled).
//!
//! HARNESS ORDERING LAW (rework after the first revision of this pin
//! failed its own shapes 1/4 on BOTH the base and head trees): an
//! at-exit existence assertion must be snapshotted in the window
//! between observing the shell's exit and draining its pipes. A
//! detached producer inherits the shell's stderr pipe and holds its
//! EOF off until the producer itself exits, so a harness that reads
//! stderr to EOF before asserting is really asserting the state at
//! PRODUCER exit — by then the producer has touched its marker and
//! every correct shell "fails". GNU bash 5.3.20 fails this pin's
//! shapes 1/4 under the drain-first ordering in exactly the same way
//! (captain probe: shell exits at 0.01s, stderr EOF at 10.01s, marker
//! present), so the snapshot below is taken the moment try_wait
//! reports the exit, and the pipes are drained on side threads whose
//! completion the assertions never wait for.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-2526c-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

struct RunOutcome {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    /// Existence of each requested marker file, snapshotted the
    /// moment the shell's exit was observed — BEFORE either pipe is
    /// drained (see the harness ordering law in the header).
    markers_at_exit: Vec<bool>,
}

/// Spawn rubash on `script` inside `dir`. `markers` names files in
/// `dir` whose existence is snapshotted at shell exit.
fn run_rubash(dir: &Path, script: &str, markers: &[&str]) -> RunOutcome {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    // Drain both pipes on side threads from the start: a detached
    // producer can hold either pipe's EOF off past the shell's exit,
    // and the main thread must be free to snapshot the markers the
    // moment the exit lands.
    let (out_tx, out_rx) = std::sync::mpsc::channel::<String>();
    if let Some(mut pipe) = child.stdout.take() {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            let _ = out_tx.send(text);
        });
    }
    let (err_tx, err_rx) = std::sync::mpsc::channel::<String>();
    if let Some(mut pipe) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            let _ = err_tx.send(text);
        });
    }
    let deadline = Instant::now() + Duration::from_secs(40);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within 40s");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    // THE snapshot: shell exit has been observed and neither pipe
    // has been waited on yet, so a still-sleeping producer has not
    // had its post-sleep act (the marker touch) counted either way.
    let markers_at_exit: Vec<bool> = markers.iter().map(|name| dir.join(name).exists()).collect();
    // Diagnostics only from here on: a pipe whose EOF is held by a
    // detached producer may not deliver until that producer exits;
    // the timeout keeps the harness bounded and the assertions above
    // do not depend on what arrives.
    let stdout = out_rx
        .recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|_| "(stdout pipe still held by a detached producer)".to_string());
    let stderr = err_rx
        .recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|_| "(stderr pipe still held by a detached producer)".to_string());
    RunOutcome {
        stdout,
        stderr,
        code: status.code(),
        markers_at_exit,
    }
}

#[test]
fn issue2526_abandoned_producer_not_waited_completion_form() {
    let dir = scratch("abandoned");
    let out = run_rubash(
        &dir.0,
        "head -c 7 <(printf 'first\\nse'; sleep 10; touch marker1); echo done1\n",
        &["marker1"],
    );
    assert_eq!(out.code, Some(0), "stderr:\n{}", out.stderr);
    assert_eq!(out.stdout, "first\nsdone1\n", "stderr:\n{}", out.stderr);
    // The producer sleeps 10s before touching the marker; the
    // snapshot was taken at shell exit, so the marker can only be
    // recorded present if the shell waited the producer out.
    assert!(
        !out.markers_at_exit[0],
        "shell waited out the abandoned producer (marker1 exists at exit)"
    );
}

#[test]
fn issue2526_bare_wait_settles_output_consumer_completion_form() {
    let dir = scratch("consumer");
    let out = run_rubash(
        &dir.0,
        "printf 'x\\n' > >(sleep 1; cat >> result.txt); wait; echo waited\n",
        &[],
    );
    assert_eq!(out.code, Some(0), "stderr:\n{}", out.stderr);
    assert_eq!(out.stdout, "waited\n", "stderr:\n{}", out.stderr);
    let result = std::fs::read_to_string(dir.0.join("result.txt")).expect("result.txt written");
    assert_eq!(result, "x\n");
}

#[test]
fn issue2526_bare_wait_waits_lone_producer_completion_form() {
    let dir = scratch("lone");
    let out = run_rubash(
        &dir.0,
        "read -n 2 x < <(printf 'ab'; sleep 2; touch marker2); echo \"x:$x\"; wait; echo done2\n",
        &["marker2"],
    );
    assert_eq!(out.code, Some(0), "stderr:\n{}", out.stderr);
    assert_eq!(out.stdout, "x:ab\ndone2\n", "stderr:\n{}", out.stderr);
    // The lone producer is the last asynchronous registration, so
    // bare `wait` settles it: its final act (the touch) has happened
    // by the time the shell exits.
    assert!(
        out.markers_at_exit[0],
        "bare wait skipped the lone producer (marker2 missing at exit)"
    );
}

#[test]
fn issue2526_bare_wait_skips_producer_when_job_owns_last_async_completion_form() {
    let dir = scratch("jobowns");
    let out = run_rubash(
        &dir.0,
        "read -n 2 y < <(printf 'cd'; sleep 6; touch marker3); echo \"y:$y\"; sleep 1 & wait; echo done3\n",
        &["marker3"],
    );
    assert_eq!(out.code, Some(0), "stderr:\n{}", out.stderr);
    assert_eq!(out.stdout, "y:cd\ndone3\n", "stderr:\n{}", out.stderr);
    // The real background job owns $!: wait returns with the job
    // (~1s), long before the producer's 6s sleep ends, so its marker
    // cannot exist at shell exit.
    assert!(
        !out.markers_at_exit[0],
        "bare wait settled the producer despite the job owning last-async (marker3 exists)"
    );
}
