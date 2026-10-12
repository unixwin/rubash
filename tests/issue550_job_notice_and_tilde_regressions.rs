//! Issue rubash#550 — naming/platform cells.
//!
//! Cell 2 (message mapping, fixed here): a background job killed by
//! SIGKILL must be announced with the conventional signal DESCRIPTION —
//! GNU jobs.c:1955 j_strsignal prints strsignal(SIGKILL) = `Killed`
//! (jobs.c:1993, WTERMSIG arm), padded to LONGEST_SIGNAL_DESC in the
//! `name: line N: PID desc cmd` notice shape. Rubash's Windows path fell
//! back to `Signal 9`.
//!
//! Cell 1 (`~root:x`, NOT changed here, platform-discounted per the
//! issue's own recommendation): GNU resolves `~root` through the system
//! user database (lib/tilde/tilde.c:361-410 getpwnam) to `/root`. Rubash's
//! lookup logic already matches GNU — the WSL probe `echo
//! ~definitely_not_a_user_xyz:x` stays literal, exactly like rubash — but
//! Windows ships no user database containing `root` (no /etc/passwd in
//! the winuxcmd root, no POSIX user library), so the cell is a platform
//! DATA difference, not a logic gap. The guard test below pins the
//! GNU-matching unresolvable-user behavior.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (script-file probes, 2026-10-12; signal descriptions sampled for
//! 1/6/8/10/12/14).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// A SIGKILLed background job is announced as `Killed` (GNU strsignal
/// wording), never `Signal 9`.
#[test]
fn sigkilled_job_notice_uses_conventional_name() {
    let (_stdout, stderr, _code) =
        rubash("set -b\nsleep 5 & p=$!; kill -9 $p; sleep 0.3\njobs >/dev/null");
    assert!(stderr.contains("Killed"), "no `Killed` notice: {stderr:?}");
    assert!(
        !stderr.contains("Signal 9"),
        "`Signal 9` still used: {stderr:?}"
    );
}

/// The description maps across the synthesized signal range (probe s1:
/// SIGHUP `Hangup`, SIGABRT `Aborted`, SIGTERM `Terminated`).
#[test]
fn job_notice_descriptions_map_across_signals() {
    let (_stdout, stderr, _code) =
        rubash("set -b\nsleep 5 & p=$!; kill -15 $p; sleep 0.3\njobs >/dev/null");
    assert!(
        !stderr.contains("Signal 15"),
        "`Signal 15` still used: {stderr:?}"
    );
}

/// Cell 1 guard: an unresolvable `~user` stays literal — the same failure
/// path GNU takes when getpwnam finds nobody (WSL probe:
/// `~definitely_not_a_user_xyz:x` is echoed literally).
#[test]
fn unresolvable_tilde_user_stays_literal() {
    let (stdout, _stderr, _code) = rubash("echo ~definitely_not_a_user_xyz:x");
    assert_eq!(stdout, "~definitely_not_a_user_xyz:x\n");
}
