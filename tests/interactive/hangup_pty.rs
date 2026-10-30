//! POSIX-pty execution-level regressions for rubash#2233: a shell
//! whose controlling terminal hangs up mid-session (pty master
//! closed) must die BY SIGHUP where GNU does — not fold the
//! terminal's EIO into a graceful EOF exit, and not orphan its jobs.
//!
//! The driver runs one wiring (session leader, stdin = pty slave,
//! stdout/stderr = pipes) through both of GNU's reader routes,
//! selected exactly as shell.c:541-547 selects them: no `-i` with
//! stderr not a tty is the NON-interactive script reader; `-i`
//! forces the interactive reader. GNU anchors (bash 5.3.20, same
//! wiring):
//!
//! - Both routes first acquire the slave as controlling terminal
//!   (general.c:651 check_dev_tty, from main shell.c:410) — without
//!   it the kernel never delivers SIGHUP on hangup at all.
//! - Script route: an untrapped shell dies by the signal (kernel
//!   default, or termsig when an EXIT trap arms the set — the trap
//!   runs, the death is still by SIGHUP); a trapped or ignored
//!   SIGHUP cannot preempt the dead terminal, so the parser's
//!   READERR arm (parse.y shell_getc, FATAL_READERROR) reports
//!   `script file read error` and exits 128.
//! - Interactive route: the termsig installation (sig.c:246/291,
//!   :315-316) catches SIGHUP; termsig_handler (sig.c:590) saves
//!   history, hangs up every job (jobs.c:1783 hangup_all_jobs),
//!   runs the EXIT trap, and dies by the signal. A trapped or
//!   ignored SIGHUP leaves the dead terminal to the reader, which
//!   folds it like EOF (graceful exit, last status).
//! - `shopt -s huponexit` in an interactive LOGIN shell hangs the
//!   jobs up on a normal exit; without it they survive
//!   (shell.c:1017-1020 exit_shell).
//!
//! Baseline (pre-fix) rubash: no controlling terminal is ever
//! acquired (tty_nr=0), so no hangup is delivered in any cell — the
//! readers fold the EIO into the last status (0/1), jobs survive
//! orphaned, and huponexit has no behavior.
//!
//! The driver skips itself (exit 2) where python3 or the pty module
//! is unavailable (Windows hosts have no SIGHUP face).

use std::process::{Command, Stdio};

const DRIVER: &str = "tests/interactive/support/pty_hangup_probe.py";

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

/// Run the named legs through the pty driver and require every one to
/// report PASS. The driver prints `leg=PASS` / `leg=FAIL detail` lines
/// and exits 2 (with a `SKIP` line) when no POSIX pty is available.
fn run_hangup_legs(legs: &[&str]) {
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
        "pty hangup legs failed (see driver output above)"
    );
    for leg in legs {
        assert!(
            stdout.contains(&format!("{leg}=PASS")),
            "leg {leg} did not report PASS (driver output above)"
        );
    }
}

/// rubash#2233, script route: closing the pty master kills the
/// shell BY SIGHUP (wait status -1) — at an idle prompt, and in the
/// #684 residual shape (a `read` that took a Ctrl-D EOF, shell
/// alive at rc 1, then the terminal hangs up).
#[test]
fn issue2233_script_route_master_close_dies_by_sighup() {
    run_hangup_legs(&["s-hangup-at-prompt", "s-hangup-after-read-eof"]);
}

/// rubash#2233, script route: a direct untrapped SIGHUP kills by
/// the signal, and a background job does not survive the hangup.
#[test]
fn issue2233_script_route_direct_kill_and_job() {
    run_hangup_legs(&["s-hangup-direct-kill", "s-hangup-kills-job"]);
}

/// rubash#2233, script route: an EXIT trap arms the termsig set —
/// on hangup the trap runs and the shell still dies by SIGHUP.
#[test]
fn issue2233_script_route_exit_trap_runs_then_dies_by_sighup() {
    run_hangup_legs(&["s-hangup-exit-trap"]);
}

/// rubash#2233, script route: trapped or ignored SIGHUP + dead
/// terminal = the fatal read-error arm (diagnostic + exit 128);
/// a trapped SIGHUP delivered while the terminal is alive still
/// runs its trap and the shell survives (control).
#[test]
fn issue2233_script_route_trapped_or_ignored_faces() {
    run_hangup_legs(&[
        "s-hangup-trapped",
        "s-hangup-ignored",
        "s-hangup-trapped-direct",
    ]);
}

/// rubash#2233, interactive route: closing the pty master kills
/// the shell BY SIGHUP — at an idle prompt, and after a read EOF.
#[test]
fn issue2233_interactive_route_master_close_dies_by_sighup() {
    run_hangup_legs(&["i-hangup-at-prompt", "i-hangup-after-read-eof"]);
}

/// rubash#2233, interactive route: hangup propagation — a
/// background job does not survive its shell's hangup
/// (jobs.c:1783 hangup_all_jobs); a direct untrapped SIGHUP takes
/// the same settlement.
#[test]
fn issue2233_interactive_route_hangup_hangs_up_jobs() {
    run_hangup_legs(&["i-hangup-kills-job", "i-hangup-direct-kill"]);
}

/// rubash#2233, interactive route: the EXIT trap runs on the
/// hangup path, in termsig order (after the job hangup), before
/// the signal death.
#[test]
fn issue2233_interactive_route_hangup_runs_exit_trap() {
    run_hangup_legs(&["i-hangup-runs-exit-trap"]);
}

/// rubash#2233, interactive route: trapped or ignored SIGHUP +
/// dead terminal folds like EOF (graceful exit, last status) —
/// not a signal death, not the script route's 128; a trapped
/// SIGHUP delivered while the terminal is alive still runs its
/// trap and the shell survives (control).
#[test]
fn issue2233_interactive_route_trapped_or_ignored_faces() {
    run_hangup_legs(&[
        "i-hangup-trapped",
        "i-hangup-ignored",
        "i-hangup-trapped-direct",
    ]);
}

/// rubash#2233: `shopt -s huponexit` in an interactive LOGIN shell
/// hangs the jobs up on a normal exit; without the shopt (control)
/// they survive (shell.c:1017-1020 exit_shell).
#[test]
fn issue2233_huponexit_login_shell_hangs_up_jobs() {
    run_hangup_legs(&["huponexit-login", "huponexit-off"]);
}
