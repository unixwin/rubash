//! Signal trap coverage regressions (rubash#1511; GNU bash 5.3.20
//! oracle-verified 2026-10-25).
//!
//! Until the fix, the kernel-signal backend managed only seven signals
//! (HUP/INT/QUIT/TERM/USR1/USR2/PIPE) plus the SIGSEGV/SIGBUS fatal
//! pair. A `trap` on any other signal changed nothing at the kernel:
//! trapped terminating signals still killed the shell by their default
//! action (the handler never ran), `trap '' SIG` never installed
//! SIG_IGN, and a trapped SIGURG/SIGWINCH was never delivered. These
//! tests pin the GNU behavior for the newly managed set: the handler
//! runs and the shell survives; the ignore form survives silently; an
//! untrapped delivery dies at 128+n; an armed EXIT trap reroutes the
//! named terminating signals (but not URG/WINCH or the realtime block,
//! which GNU's compile-time terminating set cannot name).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        output.status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        signal,
    )
}

/// The parent-observed death status of an untrapped terminating
/// delivery: GNU's parent sees 128+n whether the shell died by the
/// signal or exited with that code.
fn assert_dies_at(script: &str, n: i32) {
    let (stdout, _stderr, code, signal) = rubash(script);
    assert!(
        code == Some(128 + n) || signal == Some(n),
        "expected death at {} for {script:?}, got code={code:?} signal={signal:?} stdout={stdout:?}",
        128 + n
    );
    assert!(
        !stdout.contains("alive"),
        "shell must not survive {script:?}: {stdout:?}"
    );
}

#[cfg(target_os = "linux")]
const NEWLY_MANAGED: &[&str] = &[
    "ILL", "TRAP", "ABRT", "FPE", "ALRM", "STKFLT", "XCPU", "XFSZ", "VTALRM", "PROF", "IO", "PWR",
    "SYS", "URG", "WINCH", "RTMIN", "RTMIN+1", "RTMAX-1", "RTMAX",
];

#[cfg(not(target_os = "linux"))]
const NEWLY_MANAGED: &[&str] = &[
    "ILL", "TRAP", "ABRT", "FPE", "ALRM", "XCPU", "XFSZ", "VTALRM", "PROF", "IO", "SYS", "URG",
    "WINCH",
];

/// Trapped form: the handler runs and the shell survives (GNU installs
/// one handler per trapped signal, sig.c:830).
#[test]
fn issue_1511_trapped_signals_run_handler_and_survive() {
    for sig in NEWLY_MANAGED {
        let script = format!("trap 'echo got' {sig}; kill -{sig} $$; echo alive");
        let (stdout, _stderr, code, _signal) = rubash(&script);
        assert_eq!(
            stdout, "got\nalive\n",
            "trapped {sig} must run its handler and survive"
        );
        assert_eq!(code, Some(0), "trapped {sig} exit code");
    }
}

/// Ignore form: `trap '' SIG` installs SIG_IGN; the shell survives and
/// prints nothing for the delivery.
#[test]
fn issue_1511_ignored_signals_survive_silently() {
    for sig in NEWLY_MANAGED {
        let script = format!("trap '' {sig}; kill -{sig} $$; echo alive");
        let (stdout, _stderr, code, _signal) = rubash(&script);
        assert_eq!(stdout, "alive\n", "ignored {sig} must be survivable");
        assert_eq!(code, Some(0), "ignored {sig} exit code");
    }
}

/// Untrapped form: the kernel default action kills the shell at
/// 128+n (the pre-existing contract, extended to the new set).
#[test]
fn issue_1511_untrapped_signals_still_kill() {
    assert_dies_at("kill -ALRM $$; echo alive", 14);
    assert_dies_at("kill -ILL $$; echo alive", 4);
    assert_dies_at("kill -PROF $$; echo alive", 27);
    #[cfg(target_os = "linux")]
    assert_dies_at("kill -RTMIN $$; echo alive", 34);
}

/// Reset form: `trap - SIG` restores the default disposition, so a
/// later delivery kills again (the reconcile Default path).
#[test]
fn issue_1511_reset_restores_default_death() {
    assert_dies_at(
        "trap 'echo got' ALRM; trap - ALRM; kill -ALRM $$; echo alive",
        14,
    );
}

/// EXIT-trap reroute (sig.c:226-229): with only an EXIT trap armed, a
/// named terminating signal runs the EXIT trap and dies at 128+n.
#[test]
fn issue_1511_exit_trap_reroutes_named_terminating_signals() {
    let (stdout, _e, _c, _s) = rubash("trap 'echo exited' EXIT; kill -ALRM $$; echo alive");
    assert_eq!(stdout, "exited\n", "EXIT trap must run for untrapped ALRM");
    assert_dies_at("trap 'echo exited' EXIT; kill -ALRM $$; echo alive", 14);
}

/// EXIT-trap reroute exclusions: GNU's terminating set is a
/// compile-time table — it cannot name the realtime signals, and it
/// excludes the default-ignore pair (URG/WINCH are simply ignored).
#[test]
fn issue_1511_exit_trap_does_not_reroute_rt_or_default_ignore() {
    #[cfg(target_os = "linux")]
    {
        let (stdout, _e, _c, _s) = rubash("trap 'echo exited' EXIT; kill -RTMIN $$; echo alive");
        assert_eq!(
            stdout, "",
            "EXIT trap must NOT run for untrapped RTMIN: {stdout:?}"
        );
        assert_dies_at("trap 'echo exited' EXIT; kill -RTMIN $$; echo alive", 34);
    }
    let (stdout, _e, code, _s) = rubash("trap 'echo exited' EXIT; kill -URG $$; echo alive");
    assert_eq!(
        stdout, "alive\nexited\n",
        "URG is default-ignored; the EXIT trap runs only at normal exit: {stdout:?}"
    );
    assert_eq!(code, Some(0));
}

/// The 30-signal storm from the #1511 report, reduced to the
/// killable-in-process set: every trapped signal's handler must run
/// and the shell must survive the whole storm (pre-fix it died at
/// SIGILL, the first unmanaged delivery).
#[test]
fn issue_1511_signal_storm_completes() {
    let sigs = [
        "HUP", "INT", "QUIT", "ILL", "TRAP", "ABRT", "FPE", "USR1", "ALRM", "TERM", "STKFLT",
        "USR2", "PIPE", "XCPU", "XFSZ", "VTALRM", "PROF", "WINCH", "IO", "PWR", "SYS", "URG",
        "RTMIN", "RTMAX",
    ];
    let mut script = String::new();
    for sig in sigs {
        script.push_str(&format!("trap 'echo {sig}' {sig}; "));
    }
    for sig in sigs {
        script.push_str(&format!("kill -{sig} $$; "));
    }
    script.push_str("echo survived");
    let (stdout, _stderr, code, _signal) = rubash(&script);
    assert_eq!(code, Some(0), "storm must complete: {stdout:?}");
    for sig in sigs {
        assert!(
            stdout.lines().any(|line| line == sig),
            "storm output missing handler line for {sig}: {stdout:?}"
        );
    }
    assert!(
        stdout.contains("survived"),
        "storm survivor line: {stdout:?}"
    );
}
