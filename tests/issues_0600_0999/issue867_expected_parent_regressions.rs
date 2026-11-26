//! Issue rubash#867 regression: the PDEATHSIG fork-race check compared
//! `getppid()` against the CONSTANT 1 (`if getppid() == 1 { self-SIGKILL }`).
//! Inside a PID namespace (containers, sandboxed CI) a perfectly healthy
//! child of the namespace's PID 1 observes `getppid() == 1` before any race
//! is possible and killed itself at spawn. The fix captures the EXPECTED
//! parent pid (the spawning process's own pid) at handshake time and
//! self-kills only when the observed parent DIVERGES from it.
//!
//! The deterministic pre-fix repro needs an armed child whose parent is the
//! namespace's PID 1. Arming decisions moved in-process (rubash#866), so on
//! a fixed build a clean namespace run never arms anything; the two probes
//! here pin:
//! * L1: a wrapper-path background job under `unshare --pid --fork` (shell =
//!   namespace PID 1) runs to completion;
//! * L2: with the FORGED ambient marker (the #866 compound, the sharp edge
//!   where the pre-fix build deterministically killed every child at
//!   spawn), background children still run to completion.
//!
//! Both skip gracefully where unshare/unprivileged namespaces are
//! unavailable, so the suite stays green on hosts without them. The
//! expected-vs-observed comparison itself is pinned platform-exactly by the
//! `pdeathsig_expected_parent_tests` unit module in builtins/kill.rs.

#![cfg(target_os = "linux")]

use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::time::{Duration, Instant};

const RUN_LIMIT: Duration = Duration::from_secs(60);

fn run_bounded(cmd: &mut Command) -> (Option<i32>, String) {
    let start = Instant::now();
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn child");
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => {
                let mut stdout = String::new();
                if let Some(mut pipe) = child.stdout.take() {
                    use std::io::Read;
                    let _ = pipe.read_to_string(&mut stdout);
                }
                let code = if status.success() {
                    status.code()
                } else if let Some(sig) = status.signal() {
                    Some(128 + sig)
                } else {
                    status.code()
                };
                return (code, stdout);
            }
            None if start.elapsed() > RUN_LIMIT => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child exceeded the deadline: {cmd:?}");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn unshare_available() -> bool {
    Command::new("unshare")
        .args(["--pid", "--fork", "/bin/true"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn namespace_probe(script: &str, forge: bool) -> (Option<i32>, String) {
    let mut cmd = Command::new("unshare");
    cmd.args(["--pid", "--fork"])
        .arg(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script]);
    if forge {
        cmd.env("__RUBASH_BACKGROUND_WRAPPER", "1");
    }
    run_bounded(&mut cmd)
}

#[test]
fn l1_wrapper_job_survives_when_the_shell_is_namespace_pid1() {
    if !unshare_available() {
        eprintln!("skip: unshare unavailable or namespace creation denied");
        return;
    }
    let (code, stdout) = namespace_probe("{ sleep 2; echo BG-OK; } & wait $!; echo rc=$?", false);
    assert_eq!(code, Some(0), "stdout: {stdout}");
    assert!(stdout.contains("BG-OK"), "stdout: {stdout}");
    assert!(
        stdout.contains("rc=0"),
        "background job died at creation under PID namespace: {stdout}"
    );
}

#[test]
fn l2_forged_marker_under_namespace_pid1_kills_nothing() {
    if !unshare_available() {
        eprintln!("skip: unshare unavailable or namespace creation denied");
        return;
    }
    // Pre-fix this printed only `rc=137`: every directly-exec'd background
    // child armed PDEATHSIG from the forged env, saw getppid() == 1 (its
    // parent IS the namespace's PID 1) and SIGKILLed itself at spawn.
    let (code, stdout) = namespace_probe("sh -c 'sleep 2; echo ALIVE' & wait $!; echo rc=$?", true);
    assert_eq!(code, Some(0), "stdout: {stdout}");
    assert!(stdout.contains("ALIVE"), "stdout: {stdout}");
    assert!(
        stdout.contains("rc=0"),
        "healthy background child self-killed under PID namespace: {stdout}"
    );
}
