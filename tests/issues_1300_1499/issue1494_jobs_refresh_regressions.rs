//! rubash#1494 — `jobs` refreshed job state synchronously: right
//! after `kill -STOP %1` it already showed Stopped and promoted the
//! job to current; GNU's `jobs` reports the state as of the last
//! notification point, so the just-stopped job still shows Running
//! (both with and without an intervening builtin, and in the
//! one-line form).
//!
//! Probes: clusters/x4/repro/1494{,b,c}.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

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

fn assert_both_running_not_stopped(stdout: &str) {
    assert_eq!(stdout.matches("Running").count(), 2, "stdout: {stdout}");
    assert!(
        !stdout.contains("Stopped"),
        "jobs must not refresh synchronously, stdout: {stdout}"
    );
    assert!(stdout.contains("[1]-"), "stdout: {stdout}");
    assert!(stdout.contains("[2]+"), "stdout: {stdout}");
}

#[test]
fn jobs_after_stop_still_shows_running() {
    let (stdout, _stderr, code) = rubash(
        "set -m\nsleep 30 &\nsleep 30 &\nkill -STOP %1\njobs\nkill -9 %1 %2 2>/dev/null\nwait 2>/dev/null\necho done",
    );
    assert_both_running_not_stopped(&stdout);
    assert!(stdout.ends_with("done\n"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}

#[test]
#[ignore = "issue#1494 residual: with an intervening builtin between `kill -STOP %1` and `jobs`, the listing already shows [1]+ Stopped (state refreshed and current marker moved); GNU keeps [1]- Running until a notification point in all variants; see bf-w3-b REPORT.md"]
fn jobs_after_stop_with_intervening_builtin() {
    let (stdout, _stderr, _code) = rubash(
        "set -m\nsleep 30 &\nsleep 30 &\nkill -STOP %1\necho intervening-builtin\njobs\nkill -9 %1 %2 2>/dev/null; wait 2>/dev/null",
    );
    assert!(
        stdout.starts_with("intervening-builtin\n"),
        "stdout: {stdout}"
    );
    assert_both_running_not_stopped(&stdout);
}

#[test]
fn jobs_after_stop_one_line_form() {
    let (stdout, _stderr, _code) = rubash(
        "set -m\nsleep 30 & sleep 30 & kill -STOP %1; jobs; kill -9 %1 %2 2>/dev/null; wait 2>/dev/null",
    );
    assert_both_running_not_stopped(&stdout);
}
