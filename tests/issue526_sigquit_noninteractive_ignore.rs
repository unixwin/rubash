//! issue#526 regression: an untrapped SIGQUIT delivered to a non-interactive
//! shell must be ignored, not fatal. GNU bash ignores SIGQUIT in every shell
//! (sig.c:102 initialize_signals -> sig.c:230 initialize_shell_signals ->
//! sig.c:333 sets SIGQUIT to SIG_IGN before the interactive-only
//! SIGINT/SIGTERM installs at sig.c:335-340), so `kill -QUIT $$` leaves the
//! script running with the kill's own status, while rubash let the delivery
//! end the whole shell with rc 131 (128+SIGQUIT) and skip the rest of the
//! script. The trap.c counterpart marks SIGQUIT SIG_SPECIAL (trap.c:187-188)
//! so a `trap - QUIT` reset returns to the policy ignore, not to a killing
//! default.
//!
//! Oracle: GNU bash 5.3.0 (WSL) on 2026-10-12 — `kill -QUIT $$; sleep 0.3;
//! echo survived=$?` prints `survived=0` and exits 0; `trap - QUIT` keeps the
//! ignore; `bash -c 'kill -QUIT $$'` also survives. SIGINT keeps its killing
//! default in a non-interactive shell (verified same session: rc 130), which
//! these tests pin so the ignore cannot silently widen.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

struct Output {
    status: i32,
    stdout: String,
}

fn run_args(args: &[&str], stdin_script: Option<&str>) -> Output {
    let mut child = Command::new(shell_bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    if let Some(script) = stdin_script {
        let mut stdin = child.stdin.take().expect("stdin");
        use std::io::Write;
        stdin.write_all(script.as_bytes()).expect("write script");
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for rubash");
    Output {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
    }
}

/// Run `script` as `rubash -c <script>` with a closed stdin.
fn run_c(script: &str) -> Output {
    run_args(&["-c", script], None)
}

/// Run `script` through the stdin driver: `rubash -s` with the script fed on
/// stdin (the audited oracle entry path).
fn run_s(script: &str) -> Output {
    run_args(&["-s"], Some(script))
}

/// The issue's exact repro via -c: the shell must survive its own SIGQUIT
/// and continue with the kill's status.
#[test]
fn untrapped_sigquit_via_kill_self_is_ignored() {
    let out = run_c("kill -QUIT $$; echo survived=$?");
    assert_eq!(out.stdout, "survived=0\n", "stdout must survive SIGQUIT");
    assert_eq!(out.status, 0, "shell must exit 0 after ignoring SIGQUIT");
}

/// Same contract through the stdin driver (-s), the audited entry path.
#[test]
fn untrapped_sigquit_stdin_script_is_ignored() {
    let out = run_s("kill -QUIT $$; echo survived=$?\n");
    assert_eq!(out.stdout, "survived=0\n", "stdout must survive SIGQUIT");
    assert_eq!(out.status, 0);
}

/// A delivery landing while the shell waits for an external command must
/// also be dropped: the subshell job sends SIGQUIT to the parent's pid while
/// the parent is blocked in the external sleep.
#[test]
fn sigquit_while_blocked_in_external_wait_is_ignored() {
    let out = run_c("(kill -QUIT $$) & sleep 2; echo survived=$?");
    assert_eq!(
        out.stdout, "survived=0\n",
        "blocked shell must survive SIGQUIT"
    );
    assert_eq!(out.status, 0);
}

/// `trap - QUIT` returns SIGQUIT to the shell's policy ignore, so a later
/// delivery is still not fatal (GNU survives the same sequence).
#[test]
fn sigquit_stays_ignored_after_trap_reset() {
    let script = "trap 'echo trapped' QUIT; trap - QUIT; kill -QUIT $$; echo survived=$?";
    let out = run_c(script);
    assert_eq!(out.stdout, "survived=0\n");
    assert_eq!(out.status, 0);
}

/// A user trap overrides the policy ignore: the action must still run.
#[test]
fn trapped_sigquit_still_fires_action() {
    let out = run_c("trap 'echo trapped-ok' QUIT; kill -QUIT $$; echo after=$?");
    assert!(
        out.stdout.contains("trapped-ok"),
        "trap action must fire, got: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("after=0"),
        "script continues after trap"
    );
    assert_eq!(out.status, 0);
}

/// Guard against widening the ignore: untrapped SIGINT keeps its killing
/// default in a non-interactive shell (GNU rc 130, no further output).
#[test]
fn untrapped_sigint_still_terminates_shell() {
    let out = run_c("kill -INT $$; echo not-reached");
    assert_eq!(out.stdout, "", "nothing may run after a fatal SIGINT");
    assert_eq!(out.status, 130, "128+SIGINT");
}

/// And SIGTERM likewise stays fatal (GNU rc 143).
#[test]
fn untrapped_sigterm_still_terminates_shell() {
    let out = run_c("kill -TERM $$; echo not-reached");
    assert_eq!(out.stdout, "", "nothing may run after a fatal SIGTERM");
    assert_eq!(out.status, 143, "128+SIGTERM");
}
