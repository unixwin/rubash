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

fn run_rubash(dir: &Path, script: &str) -> (String, String, Option<i32>) {
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
    use std::io::Read;
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.code())
}

#[test]
fn issue2526_abandoned_producer_not_waited_completion_form() {
    let dir = scratch("abandoned");
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        "head -c 7 <(printf 'first\\nse'; sleep 10; touch marker1); echo done1\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "first\nsdone1\n", "stderr:\n{stderr}");
    // The producer sleeps 10s before touching the marker; the shell
    // has already exited (this assertion runs at shell exit), so the
    // marker can only exist if the shell waited the producer out.
    assert!(
        !dir.0.join("marker1").exists(),
        "shell waited out the abandoned producer (marker1 exists at exit)"
    );
}

#[test]
fn issue2526_bare_wait_settles_output_consumer_completion_form() {
    let dir = scratch("consumer");
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        "printf 'x\\n' > >(sleep 1; cat >> result.txt); wait; echo waited\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "waited\n", "stderr:\n{stderr}");
    let result = std::fs::read_to_string(dir.0.join("result.txt")).expect("result.txt written");
    assert_eq!(result, "x\n");
}

#[test]
fn issue2526_bare_wait_waits_lone_producer_completion_form() {
    let dir = scratch("lone");
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        "read -n 2 x < <(printf 'ab'; sleep 2; touch marker2); echo \"x:$x\"; wait; echo done2\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x:ab\ndone2\n", "stderr:\n{stderr}");
    // The lone producer is the last asynchronous registration, so
    // bare `wait` settles it: its final act (the touch) has happened
    // by the time the shell exits.
    assert!(
        dir.0.join("marker2").exists(),
        "bare wait skipped the lone producer (marker2 missing at exit)"
    );
}

#[test]
fn issue2526_bare_wait_skips_producer_when_job_owns_last_async_completion_form() {
    let dir = scratch("jobowns");
    let (stdout, stderr, code) = run_rubash(
        &dir.0,
        "read -n 2 y < <(printf 'cd'; sleep 6; touch marker3); echo \"y:$y\"; sleep 1 & wait; echo done3\n",
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "y:cd\ndone3\n", "stderr:\n{stderr}");
    // The real background job owns $!: wait returns with the job
    // (~1s), long before the producer's 6s sleep ends, so its marker
    // cannot exist at shell exit.
    assert!(
        !dir.0.join("marker3").exists(),
        "bare wait settled the producer despite the job owning last-async (marker3 exists)"
    );
}
