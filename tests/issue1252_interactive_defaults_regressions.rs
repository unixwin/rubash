//! Regression pins for rubash#1252: interactive shells did not apply
//! the interactive `set -o` defaults — monitor mode stayed off, `$-`
//! lacked `m`, and `fg`/`bg` failed with `no job control` even though
//! the job-control machinery existed (`set -m` by hand worked around
//! it, confirming a defaults gap, not missing machinery).
//!
//! GNU bash 5.3 (shell.c / flags: an interactive shell enables monitor
//! mode — `set -o monitor`, the `m` in `$-` — along with history and
//! histexpand; jobs.c then lets fg/bg attach). The issue's oracle was
//! GNU on a PTY: `monitor on`, `$-` = `himBH`, `sleep 30 & fg` attaches.
//! GNU only reaches that state with a terminal; without one it leaves
//! monitor off (rubash#2948, GNU 5.3.20 probed in a pipe harness:
//! `hiBHc`, monitor off, `fg: no job control`, fg_rc=1).
//!
//! Harness correction (rubash#2948): these pins originally ran the
//! interactive cases with PIPED stdio while asserting the
//! GNU-on-PTY values — the pipe harness is exactly the no-terminal
//! shape where GNU leaves monitor off, so the assertions encoded
//! the over-set bug #2948 fixes. The interactive cases now run
//! under the `script(1)` PTY harness the oracle above describes
//! (the issue1016 harness); the no-terminal interactive shape is
//! pinned by tests/issue2948_monitor_flag_regressions.rs.
//!
//!   fixed rubash -i (PTY):  `monitor on`, `$-` = himBH, fg rc 0
//!   fixed rubash -c:        `monitor off` (control — the default
//!                           must not leak into non-interactive shells)
//!
//! The histexpand/history halves of the interactive defaults are
//! rubash#1248's surface and are pinned with it; this file pins the
//! monitor half only, per the issue's own dup-check.

use std::process::Command;

fn rubash(args: &[&str], script: &str) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .arg(script)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Run `rubash -i <probe file>` under a `script(1)` PTY and return
/// the merged transcript; None when script(1) is unavailable.
#[cfg(unix)]
fn rubash_pty_interactive(probe_body: &str) -> Option<String> {
    if Command::new("script").arg("--version").output().is_err() {
        return None;
    }
    let dir = std::env::temp_dir().join(format!(
        "rubash-i1252-pty-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let probe = dir.join("probe.sh");
    std::fs::write(&probe, probe_body).expect("write probe");
    let cmdline = format!(
        "{} -i {}",
        env!("CARGO_BIN_EXE_rubash"),
        probe.to_string_lossy()
    );
    let out = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("spawn script(1)");
    let _ = std::fs::remove_dir_all(&dir);
    Some(String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr))
}

#[cfg(unix)]
#[test]
fn issue1252_interactive_enables_monitor_and_m_flag() {
    let Some(transcript) = rubash_pty_interactive("set -o | grep -E '^monitor'\necho FLAGS=$-\n")
    else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    assert!(
        transcript
            .lines()
            .any(|l| l.trim_end().starts_with("monitor") && l.trim_end().ends_with("on")),
        "interactive shell on a terminal must default monitor on, got: {transcript:?}"
    );
    let flags = transcript
        .lines()
        .find(|l| l.contains("FLAGS="))
        .unwrap_or("");
    assert!(
        flags.contains("FLAGS=himBH"),
        "$- must carry m in interactive mode on a terminal (GNU-on-PTY: himBH), got: {flags:?}"
    );
}

#[cfg(unix)]
#[test]
fn issue1252_interactive_fg_attaches_without_set_m() {
    let Some(transcript) = rubash_pty_interactive("sleep 0.5 & fg; echo fg_rc=$?\n") else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    assert!(
        transcript.contains("fg_rc=0"),
        "fg must attach to the running job interactively, got transcript: {transcript:?}"
    );
    assert!(
        !transcript.contains("no job control"),
        "the 'no job control' failure was the bug, got transcript: {transcript:?}"
    );
}

#[test]
fn issue1252_noninteractive_monitor_stays_off_control() {
    let (stdout, _e, _c) = rubash(&["-c"], "set -o | grep -E '^monitor'");
    assert!(
        stdout
            .lines()
            .any(|l| l.starts_with("monitor") && l.ends_with("off")),
        "non-interactive shells keep monitor off, got: {stdout:?}"
    );
}
