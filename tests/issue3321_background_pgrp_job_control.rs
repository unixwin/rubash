//! rubash#3321 — job-control availability in a background process
//! group that HAS a controlling terminal.
//!
//! GNU anchor (jobs.c:4731-4869 `initialize_job_control`, bash
//! 5.3.20): the shell takes shell_tty (dup(stderr), or open
//! ("/dev/tty") when forced_interactive and stderr is not a tty),
//! then loops on tcgetpgrp — while it is not the terminal's
//! foreground process group it restores SIGTTIN to DFL and stops
//! itself (jobs.c:4786-4810), and after the 18th kill
//! (`tty_sigs++ > 16`) it bails with
//! "initialize_job_control: no job control in background" plus
//! `m` dropped, WITHOUT the second "no job control in this shell"
//! line (jobs.c:4797 `goto just_bail` skips it). A shell that only
//! asks "is stderr a tty / does /dev/tty open" never stops and
//! never reports the failure, so the background group was
//! misjudged as job-control-available (pre-fix rubash: no stop,
//! FLAGS=himBHc on all three background faces).
//!
//! The gate note on the issue demands the ctty-carrying harness
//! first: the shape is unreachable in a sealed no-ctty test
//! environment, so the driver
//! (`tests/interactive/support/pty_background_pgrp_probe.py`)
//! builds it — one fresh pty session per leg, a session-leader
//! child that owns the pty as its controlling terminal and stays
//! the foreground process group, the shell under test forked into
//! a NEW (background) process group of that session, stdio on the
//! slave. Measured against GNU bash 5.3.20 (oracle:
//! `~/workspace/rubash-work/worktrees/issue-3308/third_party/bash/bash`):
//! all four legs PASS there, and 1/4 (fg-control only) on the
//! unfixed tree.
//!
//! The driver skips itself (exit 2) where python3 or the pty module
//! is unavailable.

use std::process::{Command, Stdio};

const DRIVER: &str = "tests/interactive/support/pty_background_pgrp_probe.py";

fn python_interpreter() -> Option<String> {
    for candidate in ["python3", "python"] {
        let ok = Command::new(candidate)
            .arg("-c")
            .arg("import pty")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if ok {
            return Some(candidate.to_string());
        }
    }
    None
}

/// Run the named legs through the pty driver and require every one
/// to report PASS. The driver prints `leg=PASS` / `leg=FAIL detail`
/// lines and exits 2 (with a `SKIP` line) when no POSIX pty is
/// available.
fn run_pty_legs(legs: &[&str]) {
    let Some(python) = python_interpreter() else {
        eprintln!("skip: python3 with pty module not available");
        return;
    };
    let mut cmd = Command::new(&python);
    cmd.arg(env!("CARGO_MANIFEST_DIR").to_string() + "/" + DRIVER)
        .arg(env!("CARGO_BIN_EXE_rubash"))
        .args(legs)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd
        .output()
        .expect("failed to spawn the pty driver (is python3 on PATH?)");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    eprintln!("driver output:\n{stdout}");
    if !stderr.trim().is_empty() {
        eprintln!("driver stderr:\n{stderr}");
    }
    if stdout.trim_start().starts_with("SKIP") {
        eprintln!("skip: pty driver reports unavailable dependencies");
        return;
    }
    assert!(
        output.status.success(),
        "pty background-pgrp legs failed (see driver output above)"
    );
    for leg in legs {
        assert!(
            stdout.contains(&format!("{leg}=PASS")),
            "leg {leg} did not report PASS (driver output above)"
        );
    }
}

/// Background group, SIGTTIN default: the shell must STOP with
/// SIGTTIN at startup, and after the harness foregrounds it and
/// SIGCONTs it, it runs with job control (FLAGS=himBHc) and no
/// diagnostics.
#[test]
fn background_group_stops_until_foregrounded() {
    run_pty_legs(&["bg-stop-then-fg"]);
}

/// Background group, SIGTTIN blocked: the shell must NOT stop; it
/// bails with the background diagnostic only (no second
/// "no job control in this shell" line) and FLAGS=hiBHc.
#[test]
fn background_group_blocked_sigttin_bails_without_job_control() {
    run_pty_legs(&["bg-sigttin-blocked"]);
}

/// Background group with stderr piped (not a tty): the stop-then-fg
/// shape still holds — GNU reaches the terminal via /dev/tty in the
/// forced_interactive arm (jobs.c:4755-4766).
#[test]
fn background_group_with_piped_stderr_stops_until_foregrounded() {
    run_pty_legs(&["bg-stderr-pipe"]);
}

/// Control: a foreground-group shell on the same pty establishes job
/// control immediately, with no stop and no diagnostics.
#[test]
fn foreground_group_control_keeps_job_control() {
    run_pty_legs(&["fg-control"]);
}
