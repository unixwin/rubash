//! Issue rubash#866 regression: the background-wrapper marker used to be the
//! plain, user-guessable assignment
//! `__RUBASH_BACKGROUND_WRAPPER=1` (compound_exec) checked verbatim by
//! `child_pre_exec_reset` (builtins/kill.rs). Any script could
//! `export __RUBASH_BACKGROUND_WRAPPER=1` and thereby impose the wrapper's
//! PR_SET_PDEATHSIG semantics on every child the shell spawned afterwards —
//! including ordinary third-party programs that never went through the
//! wrapper spawn path.
//!
//! The fix replaces the forgeable constant with an unguessable spawn-time
//! nonce carried by TWO channels (the env key and a consumed
//! `--internal-bg-wrapper=` argv flag); the wrapper role arms in-process
//! only when the channels agree, and the env key is scrubbed at startup.
//!
//! Pinned here:
//! * clean environments keep working (W0);
//! * the #737 contract survives — a SIGKILLed wrapper still takes its child
//!   down (W1);
//! * a FORGED ambient value neither kills off direct-path background
//!   children at shell exit (W2, the issue repro — meaningful on Linux,
//!   where the arming exists) nor disturbs a wrapper-path job (W3).
//!
//! Every run is deadline-bounded: the pre-fix failure mode of W2 is a child
//! killed at parent exit, never a hang.

use std::process::Command;
use std::time::{Duration, Instant};

const RUN_LIMIT: Duration = Duration::from_secs(60);

struct RunOutcome {
    timed_out: bool,
    stdout: String,
}

fn run_rubash_script(script: &str, forge_env: bool) -> RunOutcome {
    let exe = env!("CARGO_BIN_EXE_rubash");
    let mut cmd = Command::new(exe);
    cmd.arg("-c").arg(script);
    if forge_env {
        cmd.env("__RUBASH_BACKGROUND_WRAPPER", "1");
    }
    let start = Instant::now();
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    // Bounded wait, then hard-kill the tree (the discipline: tests only ever
    // kill processes this test created).
    loop {
        match child.try_wait().expect("try_wait rubash") {
            Some(status) => {
                let mut stdout = String::new();
                if let Some(mut pipe) = child.stdout {
                    use std::io::Read;
                    let _ = pipe.read_to_string(&mut stdout);
                }
                assert!(status.success(), "rubash -c failed: {status}");
                return RunOutcome {
                    timed_out: false,
                    stdout,
                };
            }
            None if start.elapsed() > RUN_LIMIT => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                panic!("script exceeded the deadline: {script}");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// A child command that, if it survives being orphaned, prints
/// ORPHAN-SURVIVED after ~2.5s (unix external; the arming probes below are
/// unix-only).
#[cfg(unix)]
fn survivor_snippet() -> &'static str {
    "sh -c 'sleep 2.5; echo ORPHAN-SURVIVED'"
}

#[test]
fn w0_clean_env_wrapper_job_still_runs() {
    let out = run_rubash_script(
        "echo START; { echo WRAPPER-OK; } & wait $!; echo END",
        false,
    );
    assert!(!out.timed_out);
    assert!(out.stdout.contains("START"));
    assert!(out.stdout.contains("WRAPPER-OK"), "stdout: {}", out.stdout);
    assert!(out.stdout.contains("END"));
}

// unix only: the dies-with-the-wrapper contract is delivered by
// PR_SET_PDEATHSIG on Linux (verified end-to-end in WSL against the GNU
// oracle). Windows never had this mechanism for a wrapper's plain fg child
// — the pre-fix master binary shows the same ORPHAN-SURVIVED — its #737
// coverage rides the job-object parent tracking pinned by its own tests.
#[cfg(unix)]
#[test]
fn w1_killing_the_wrapper_still_kills_its_child_737_contract() {
    // The wrapper's fg child (the survivor) must die with the wrapper when
    // the wrapper is SIGKILLed — otherwise `kill $!` leaves the real work
    // orphaned and ORPHAN-SURVIVED appears after 2.5s.
    let script = format!(
        "{{ {survivor}; }} & job=$!; sleep 0.6; kill -9 $job; wait $job; rc=$?; \
         echo killed-rc=$rc; sleep 4",
        survivor = survivor_snippet(),
    );
    let out = run_rubash_script(&script, false);
    assert!(out.stdout.contains("killed-rc="), "stdout: {}", out.stdout);
    assert!(
        !out.stdout.contains("ORPHAN-SURVIVED"),
        "wrapper child survived the wrapper kill (#737 regression): {}",
        out.stdout
    );
}

#[cfg(target_os = "linux")]
#[test]
fn w2_forged_env_does_not_arm_pdeathsig_on_direct_children() {
    // The issue-866 repro: with the forged value, the directly-exec'd async
    // child (never spawned through the wrapper path) used to inherit the
    // arming and die the moment the shell exited. GNU semantics (and the
    // fixed shell): it outlives the shell and prints.
    let out = run_rubash_script(
        &format!("{survivor} & exit 0", survivor = survivor_snippet(),),
        true,
    );
    assert!(
        out.stdout.contains("ORPHAN-SURVIVED"),
        "forged __RUBASH_BACKGROUND_WRAPPER killed an unrelated background child at shell exit: {:?}",
        out.stdout
    );
}

#[test]
fn w3_forged_env_does_not_disturb_a_wrapper_job() {
    let out = run_rubash_script("{ echo W3-OK; } & wait $!; echo W3-END", true);
    assert!(out.stdout.contains("W3-OK"), "stdout: {}", out.stdout);
    assert!(out.stdout.contains("W3-END"));
}
