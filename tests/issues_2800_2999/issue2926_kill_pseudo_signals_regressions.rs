//! rubash#2926: `kill -l/-s` reject the trap pseudo-signals
//! ERR/DEBUG/RETURN; GNU maps them to 66/65/67.
//!
//! GNU ground truth (bash 5.3.20):
//! - trap.h:40-46: DEBUG_TRAP = NSIG (65), ERROR_TRAP = NSIG+1 (66),
//!   RETURN_TRAP = NSIG+2 (67); support/signames.c:466-468 puts the
//!   bare names in `signal_names[]`, so trap.c:236 `decode_signal`
//!   resolves bare `ERR`/`DEBUG`/`RETURN` (no `SIG` prefix) to
//!   66/65/67, while `SIGERR` etc. do not match.
//! - builtins/common.c:747 `display_signal_list`: the full `kill -l`
//!   list iterates 1..NSIG (pseudo-signals excluded) and a numeric
//!   word requires `lsignum < NSIG`, so `kill -l 65/66/67` is an
//!   invalid specification.
//! - builtins/kill.def:194-216: delivery parses the pid first, then
//!   kill(2); ESRCH beats EINVAL — a dead pid reports "No such
//!   process" for `-s ERR`, while a live pid reports the invalid
//!   signal specification. The kernel itself returns ESRCH for
//!   kill(999999, 66) and EINVAL for kill(own-pid, 66).
//! - `EXIT` maps to 0 in both shells (already correct).

use std::process::{Command, Stdio};

fn run_shell(body: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `kill -l` maps the bare pseudo-signal names to 66/65/67/0.
#[test]
fn kill_list_maps_pseudo_signals_to_numbers() {
    for (name, want) in [
        ("ERR", "66"),
        ("DEBUG", "65"),
        ("RETURN", "67"),
        ("EXIT", "0"),
    ] {
        let (stdout, stderr, rc) = run_shell(&format!("kill -l {name}"));
        assert_eq!(rc, Some(0), "kill -l {name}: stderr {stderr:?}");
        assert_eq!(stdout, format!("{want}\n"), "kill -l {name}");
    }
}

/// The `SIG`-prefixed spellings do NOT map (the table entries carry
/// no `SIG` prefix) — they stay invalid specifications, rc 1.
#[test]
fn kill_list_sig_prefixed_pseudo_signals_stay_invalid() {
    for name in ["SIGERR", "SIGDEBUG", "SIGRETURN"] {
        let (stdout, stderr, rc) = run_shell(&format!("kill -l {name}"));
        assert_eq!(rc, Some(1), "kill -l {name} should fail: {stdout:?}");
        assert!(
            stderr.contains("invalid signal specification"),
            "kill -l {name}: expected invalid-spec diagnostic: {stderr:?}"
        );
    }
}

/// Numeric pseudo-signal values are out of range for `kill -l`
/// (common.c requires lsignum < NSIG).
#[test]
fn kill_list_numeric_pseudo_signals_stay_invalid() {
    for n in ["65", "66", "67"] {
        let (_, stderr, rc) = run_shell(&format!("kill -l {n}"));
        assert_eq!(rc, Some(1), "kill -l {n} should fail");
        assert!(
            stderr.contains("invalid signal specification"),
            "kill -l {n}: expected invalid-spec diagnostic: {stderr:?}"
        );
    }
}

/// Dead pid + `-s ERR/DEBUG/RETURN`: ESRCH beats EINVAL — "No such
/// process", rc 1 (same as `-s EXIT` on a dead pid).
#[test]
fn kill_signal_dead_pid_reports_no_such_process() {
    for sig in ["ERR", "DEBUG", "RETURN", "EXIT"] {
        let (_, stderr, rc) = run_shell(&format!("kill -s {sig} 999999"));
        assert_eq!(rc, Some(1), "kill -s {sig} 999999 should fail");
        assert!(
            stderr.contains("No such process"),
            "kill -s {sig} 999999: expected ESRCH message: {stderr:?}"
        );
    }
}

/// Live pid + `-s ERR/DEBUG/RETURN`: EINVAL — invalid specification,
/// rc 1, and the job survives.
#[test]
fn kill_signal_live_pid_reports_invalid_spec_and_job_survives() {
    for sig in ["ERR", "DEBUG", "RETURN"] {
        let (stdout, stderr, rc) = run_shell(&format!(
            "sleep 3 & p=$!; kill -s {sig} $p; echo rc=$?; \
             kill -0 $p 2>/dev/null && echo ALIVE; kill -KILL $p; wait $p; echo w=$?"
        ));
        assert_eq!(rc, Some(0));
        assert!(
            stdout.contains("w=137"),
            "cleanup wait should report 137: {stdout:?}"
        );
        assert!(
            stdout.contains("rc=1"),
            "kill -s {sig} on a live pid should fail: {stdout:?}"
        );
        assert!(
            stderr.contains("invalid signal specification"),
            "kill -s {sig}: expected invalid-spec diagnostic: {stderr:?}"
        );
        assert!(
            stdout.contains("ALIVE"),
            "kill -s {sig} must not kill the live job: {stdout:?}"
        );
    }
}
