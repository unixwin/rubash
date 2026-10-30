//! rubash#1034: a trapped signal delivered while the shell is blocked in
//! a foreground external command defers until that command completes.
//!
//! GNU jobs.c:3064 wait_for installs wait_sigint_handler while the
//! foreground child runs, which only records wait_sigint_received — it
//! does NOT set pending_traps (trap.c:545 trap_handler is replaced for
//! the wait). A shell-directed delivery (`kill -INT $$`) leaves the
//! child alive, and the trap dispatches after wait_for returns, at the
//! next command boundary (execute_cmd.c:643 run_pending_traps;
//! trap.c:328 run_pending_traps / trap.c:670 check_signals) — bash
//! 5.3.20 prints GOTINT only after the full `sleep 10` (rc 41).
//! The in-process `sleep` fast path must defer the same way instead of
//! dispatching mid-sleep.
//!
//! All expectations byte-verified against bash 5.3.20 via script files.

use std::process::Command;
use std::time::{Duration, Instant};

fn run_rubash(script: &str) -> (String, String, Option<i32>, Duration) {
    let start = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("spawn rubash");
    let elapsed = start.elapsed();
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        elapsed,
    )
}

/// Shell-directed INT during a foreground `sleep` waits out the sleep:
/// the trap fires at the boundary after, the sleep itself exits 0, and
/// the wall clock covers the whole sleep (old immediate-dispatch build
/// ended the sleep at ~0.3s with rc 130).
#[test]
fn trapped_int_defers_until_sleep_completes() {
    let (stdout, stderr, code, elapsed) = run_rubash(
        "trap 'echo GOTINT' INT\n\
         (sleep 0.3; kill -INT $$) &\n\
         sleep 2\n\
         echo \"after rc=$?\"",
    );
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert!(
        stdout.contains("GOTINT\n"),
        "trap must still fire, at the boundary after sleep:\n{stdout}"
    );
    assert!(
        stdout.contains("after rc=0\n"),
        "sleep runs to completion (not 128+sig):\n{stdout}"
    );
    assert_eq!(code, Some(0));
    assert!(
        elapsed >= Duration::from_millis(1500),
        "trap deferred past the sleep (elapsed {elapsed:?}):\n{stdout}"
    );
}

/// The issue repro shape (shortened sleeps): `exit` in the deferred
/// action still ends the shell with its status after the sleep.
#[test]
fn deferred_int_exit_action_runs_after_sleep() {
    let (stdout, stderr, code, elapsed) = run_rubash(
        "trap 'echo GOTINT; exit 41' INT\n\
         (sleep 0.3; kill -INT $$) &\n\
         sleep 3",
    );
    assert_eq!(stderr, "", "stderr: {stderr:?}");
    assert_eq!(stdout, "GOTINT\n");
    assert_eq!(code, Some(41));
    assert!(
        elapsed >= Duration::from_millis(2000),
        "trap deferred past the sleep (elapsed {elapsed:?}):\n{stdout}"
    );
}
