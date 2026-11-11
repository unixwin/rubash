//! rubash#2526 — procsub boundary/job-table settlement membership.
//!
//! Adjudication history (captain, 2026-11-02, on the issue): the
//! headline 10.3s block was the issue1959 pin's own harness measuring
//! pipe-drain time (the abandoned producer inherits the harness
//! pipes, so read_to_end waits it out for ANY shell, GNU included) —
//! the Linux shell face had no defect, and the pin was repaired in
//! PR #2567 to clock spawn→shell-exit before draining. The Windows
//! finish-time seek face was handed to the Windows side. What
//! remained open in the issue text was the related observation to
//! check in the same pass: bare `wait` did not settle output-form
//! procsub consumers — the settlement-membership question (GNU
//! jobs.c:2796 wait_for_background_pids: a procsub child is waited
//! only as the LAST asynchronous pid — process_substitute registers
//! its child there, subst.c:6362 — and a later `cmd &` overwrites
//! that registration).
//!
//! These pins lock the three membership cells, measured against GNU
//! bash 5.3.20 on master 88047f50 (2026-11-08), where all three
//! already match GNU — they are confirmation pins for the
//! adjudicated semantics, NOT fail-first pins: their job is to stop
//! a future "fix" from breaking the membership rule in either
//! direction (waiting producers that must be abandoned, or dropping
//! consumers that must be settled).
//!
//! Timing methodology is the repaired #1959 one: the clock is
//! spawn→shell-exit observed at try_wait, BEFORE the harness drains
//! its pipes. Bounds are wide (producer sleeps 3-6s, thresholds at
//! roughly half) so loaded CI cannot flip them.

use std::io::Read;
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
    let dir = std::env::temp_dir().join(format!("rubash-2526-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

fn tool_path() -> String {
    if cfg!(windows) {
        std::env::var("PATH").unwrap_or_default()
    } else {
        "/usr/bin:/bin".to_string()
    }
}

/// Spawn rubash on `script` inside `dir`; the returned Duration is
/// spawn→shell-exit at the try_wait hit, before pipe drain (see the
/// file header for why that is the only honest clock here).
fn run_rubash(
    dir: &Path,
    script: &str,
    limit: Duration,
) -> (String, String, Option<i32>, Duration) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", tool_path())
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cfg!(windows) {
        for name in [
            "SystemRoot",
            "SystemDrive",
            "COMSPEC",
            "PATHEXT",
            "OS",
            "TEMP",
            "TMP",
        ] {
            if let Ok(value) = std::env::var(name) {
                cmd.env(name, value);
            }
        }
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    let spawned = Instant::now();
    let deadline = spawned + limit;
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within {limit:?}");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let exit_elapsed = spawned.elapsed();
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.code(), exit_elapsed)
}

/// GNU: bare `wait` settles an output-form procsub consumer — the
/// consumer is the last asynchronous registration, so `wait` blocks
/// until it has drained its input and finished (here: appended the
/// byte to result.txt). The file must be complete AT shell exit,
/// not at some later orphan drain.
#[test]
fn issue2526_bare_wait_settles_output_procsub_consumer() {
    let dir = scratch("consumer");
    let (stdout, stderr, code, elapsed) = run_rubash(
        &dir.0,
        "printf 'x\\n' > >(sleep 3; cat >> result.txt); wait; echo waited\n",
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "waited\n", "stderr:\n{stderr}");
    assert!(
        elapsed >= Duration::from_millis(2000),
        "bare wait returned after {elapsed:?} without settling the consumer"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "bare wait over-waited: {elapsed:?}"
    );
    let result = std::fs::read_to_string(dir.0.join("result.txt")).expect("result.txt written");
    assert_eq!(result, "x\n");
}

/// GNU: a lone abandoned INPUT producer IS waited by bare `wait` —
/// process_substitute left its pid as the last asynchronous one and
/// nothing overwrote it (jobs.c:2796). Counter-intuitive, but it is
/// the oracle's measured shape (10.1s on the sleep-10 form); a
/// settlement that never waits producers breaks this cell.
#[test]
fn issue2526_bare_wait_waits_lone_abandoned_producer() {
    let dir = scratch("lone-producer");
    let (stdout, stderr, code, elapsed) = run_rubash(
        &dir.0,
        "read -n 2 x < <(printf 'ab'; sleep 3); echo \"x:$x\"; wait; echo done\n",
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x:ab\ndone\n", "stderr:\n{stderr}");
    assert!(
        elapsed >= Duration::from_millis(2000),
        "bare wait skipped the lone producer after {elapsed:?} (GNU waits it)"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "bare wait over-waited: {elapsed:?}"
    );
}

/// GNU: once a real background job owns $!, the abandoned producer
/// is out of the wait set — bare `wait` returns with the job (≈1s),
/// not with the producer (6s), and the shell exits without draining
/// the producer either.
#[test]
fn issue2526_bare_wait_skips_producer_when_job_owns_last_async() {
    let dir = scratch("job-owns");
    let (stdout, stderr, code, elapsed) = run_rubash(
        &dir.0,
        "read -n 2 x < <(printf 'ab'; sleep 6); echo \"x:$x\"; sleep 1 & wait; echo done\n",
        Duration::from_secs(30),
    );
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "x:ab\ndone\n", "stderr:\n{stderr}");
    assert!(
        elapsed < Duration::from_millis(4000),
        "shell blocked {elapsed:?} on the abandoned producer (GNU returns with the job)"
    );
}
