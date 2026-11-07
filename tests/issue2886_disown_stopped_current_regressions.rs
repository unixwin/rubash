//! rubash#2886: bare `disown` on a ^Z-stopped current job reports
//! "no such job"; GNU silently removes it (with the stopped-job
//! warning).
//!
//! GNU ground truth (bash 5.3.20): bare `disown` resolves the
//! CURRENT job — builtins/jobs.def `disown_builtin` calls
//! `get_job_spec(list)` with an empty list, and
//! builtins/common.c:691 returns `js.j_current` for that; the job is
//! then deleted with `delete_job(job, DEL_WARNSTOPPED)` (jobs.c:1495),
//! which prints `warning: deleting stopped job N with process group
//! M` and rc 0. The stopped job became current at stop time
//! (jobs.c:4225-4232 `set_current_job(last_stopped_job)`).
//!
//! Rubash's `disown_current_job` (job_builtins.rs) resolved the
//! "current" job through `last_background_pid` — set only for `&`
//! spawns, never for a ^Z-stopped FOREGROUND job — so the lookup
//! failed even though the table's `current_job()` already pointed at
//! the stopped job (table.rs `mark_stopped` calls `set_current_job`).
//!
//! The pin drives an interactive PTY session through `script(1)`
//! (the rubash#1016 harness shape): `sleep 30`, ^Z, `jobs`,
//! `disown`, `jobs`. Interactive-only: the non-interactive `-c`
//! shape never lands a job in the stopped-current state.

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn run_pty_session(lines: &[&str]) -> String {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("no script(1) on this host; skipping");
        return String::new();
    }
    let cmdline = format!("{} -i", env!("CARGO_BIN_EXE_rubash"));
    let mut child = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .env("TERM", "dumb")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn script(1)");

    let mut stdin = child.stdin.take().expect("child stdin");
    let owned_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    let writer = std::thread::spawn(move || {
        for line in owned_lines {
            if stdin.write_all(line.as_bytes()).is_err() {
                break;
            }
            let _ = stdin.flush();
            std::thread::sleep(Duration::from_secs(1));
        }
    });

    let mut stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    let buf = match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(buf) => buf,
        Err(_) => {
            let _ = child.kill();
            panic!("interactive session did not finish within 30s");
        }
    };
    let _ = child.wait();
    let _ = writer.join();
    let _ = reader.join();
    String::from_utf8_lossy(&buf).into_owned()
}

/// Bare `disown` removes the ^Z-stopped current job: the DEL_WARNSTOPPED
/// warning prints, the second `jobs` is empty, and `disown` itself
/// reports no "no such job". The trailing `kill %1` probe MUST report
/// "no such job" — that is the proof the job is gone (the fail-first
/// draft asserted the phrase nowhere in the transcript, which the
/// probe itself violates; the assertion is scoped to before the probe).
/// `\x1a` is ^Z.
#[test]
fn bare_disown_removes_stopped_current_job() {
    let text = run_pty_session(&[
        "sleep 30\n",
        "\x1a",
        "jobs\n",
        "disown\n",
        "jobs\n",
        "echo RC=$?\n",
        "kill %1\n",
        "exit\n",
        "exit\n",
    ]);
    if text.is_empty() {
        return;
    }
    assert!(
        text.contains("warning: deleting stopped job 1 with process group"),
        "GNU prints the stopped-job warning on bare disown: {text:?}"
    );
    let (before_kill, after_kill) = text.split_once("kill %1").unwrap_or((text.as_str(), ""));
    assert!(
        !before_kill.contains("no such job"),
        "bare disown must not report 'no such job': {text:?}"
    );
    assert!(
        after_kill.contains("no such job"),
        "kill %1 after the disown must fail: the job is gone: {text:?}"
    );
    // "Stopped" appears exactly twice: the ^Z stop notice and the
    // first `jobs` listing. The second `jobs` (after disown) must
    // print nothing.
    assert_eq!(
        text.matches("Stopped").count(),
        2,
        "the second jobs listing must be empty: {text:?}"
    );
}

/// Control: bare `disown` with a RUNNING background job removes it
/// silently (no warning) — the non-PTY path, which already worked.
#[test]
fn bare_disown_removes_running_background_job_silently() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("sleep 5 & disown; echo rc=$?; jobs; echo END")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout.contains("rc=0"),
        "bare disown should succeed: {stdout:?}"
    );
    assert!(
        stdout.contains("END") && !stdout.contains("Running"),
        "the job must be gone from the table: {stdout:?}"
    );
    assert!(
        !stderr.contains("warning:"),
        "no stopped-job warning for a running job: {stderr:?}"
    );
}

/// Control: bare `disown` with no jobs at all errors rc 1.
#[test]
fn bare_disown_with_no_jobs_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("disown; echo rc=$?")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stdout.contains("rc=1"),
        "bare disown with no jobs: {stdout:?}"
    );
    assert!(
        stderr.contains("disown: current: no such job"),
        "expected the no-such-job diagnostic: {stderr:?}"
    );
}
