//! rubash#1552 — a timed pipeline whose first stage is a dash word
//! lost its `command not found` diagnostic. Filed form (GNU bash
//! 5.3.20 prints BOTH diagnostics, the timing, and exits 127):
//!
//!     PATH=/nonexistent; time -p '-p' | cat
//!
//! Root cause (found by widening the probe): the loss needs an
//! AMBIENT `2>&1` binding around the pipeline — the parsetime2
//! comparison harness wrapped cases in `( eval ... ) 2>&1 | sed`,
//! which is how it measured the loss while bare invocations passed.
//! GNU binds a pipeline element's fd 2 to the fd-1 object the
//! pipeline inherited (execute_cmd.c:2620 execute_pipeline connects
//! the elements with fresh pipes first; an enclosing `2>&1` dups
//! onto the ENCLOSING stream). rubash's sequential pipeline driver
//! folded every stage's captured stderr into the inter-stage pipe
//! payload whenever fd 2 resolved to Stdout, so a non-final stage's
//! diagnostic became the next stage's stdin and died unread — for
//! dash words and ordinary names alike, timed or not. The fix routes
//! ambient-binding stderr of non-final stages to the pipeline's own
//! output stream in stage order; a stage's OWN `2>&1` still resolves
//! against its pipe-bound fd 1 (route_pipeline_stage_streams).
//!
//! Expectations are the GNU oracle's, probed from script files.

use std::process::Command;

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1552_timed_dash_first_stage_under_ambient_dup() {
    let (stdout, stderr, code) =
        rubash_c("PATH=/nonexistent; { time '-p' true | cat; } 2>&1; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let dash = stdout
        .find("-p: command not found")
        .expect("stdout missing the -p diagnostic: {stdout}");
    let cat = stdout
        .find("cat: command not found")
        .expect("stdout missing the cat diagnostic: {stdout}");
    assert!(dash < cat, "diagnostics out of order: {stdout}");
    assert!(stdout.contains("real"), "missing time report: {stdout}");
    assert!(stdout.contains("rc=127"), "stdout: {stdout}");
    assert!(!stderr.contains("command not found"), "stderr: {stderr}");
}

#[test]
fn issue1552_timed_posix_dash_operand_under_ambient_dup() {
    let (stdout, _stderr, code) =
        rubash_c("PATH=/nonexistent; { time -p '-p' | cat; } 2>&1; echo rc=$?");
    assert_eq!(code, Some(0), "stdout: {stdout}");
    assert!(stdout.contains("-p: command not found"), "stdout: {stdout}");
    assert!(
        stdout.contains("cat: command not found"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("real"), "missing time report: {stdout}");
    assert!(stdout.contains("rc=127"), "stdout: {stdout}");
}

#[test]
fn issue1552_timed_ordinary_first_stage_under_ambient_dup() {
    let (stdout, _stderr, code) =
        rubash_c("PATH=/nonexistent; { time nosuch1552cmd | cat; } 2>&1; echo rc=$?");
    assert_eq!(code, Some(0), "stdout: {stdout}");
    assert!(
        stdout.contains("nosuch1552cmd: command not found"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("cat: command not found"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("rc=127"), "stdout: {stdout}");
}

#[test]
fn issue1552_all_stages_diagnose_under_ambient_dup() {
    let (stdout, stderr, code) =
        rubash_c("PATH=/nonexistent; { nosuch1552a | nosuch1552b | nosuch1552c; } 2>&1");
    assert_eq!(code, Some(127), "stdout: {stdout}");
    let a = stdout
        .find("nosuch1552a: command not found")
        .expect("stdout: {stdout}");
    let b = stdout
        .find("nosuch1552b: command not found")
        .expect("stdout: {stdout}");
    let c = stdout
        .find("nosuch1552c: command not found")
        .expect("stdout: {stdout}");
    assert!(a < b && b < c, "diagnostics out of order: {stdout}");
    assert!(!stderr.contains("command not found"), "stderr: {stderr}");
}

#[test]
fn issue1552_external_prefix_middle_stage_under_ambient_dup() {
    let (stdout, _stderr, code) = rubash_c(
        "PATH=/nonexistent; { /usr/bin/true | nosuch1552mid | /usr/bin/true; } 2>&1; echo rc=$?",
    );
    assert_eq!(code, Some(0), "stdout: {stdout}");
    assert!(
        stdout.contains("nosuch1552mid: command not found"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("rc=0"), "stdout: {stdout}");
}

#[test]
fn issue1552_comsub_capture_idiom_keeps_diagnostics() {
    let (stdout, stderr, code) =
        rubash_c("x=$( { PATH=/nonexistent; nosuch1552a | nosuch1552b; } 2>&1 ); echo \"[$x]\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("nosuch1552a: command not found"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("nosuch1552b: command not found"),
        "stdout: {stdout}"
    );
}

#[test]
fn issue1552_no_redirect_control_diagnostics_on_stderr() {
    let (stdout, stderr, code) = rubash_c("PATH=/nonexistent; nosuch1552a | nosuch1552b");
    assert_eq!(code, Some(127), "stdout: {stdout}");
    assert!(
        stderr.contains("nosuch1552a: command not found"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("nosuch1552b: command not found"),
        "stderr: {stderr}"
    );
    assert!(!stdout.contains("command not found"), "stdout: {stdout}");
}

#[test]
fn issue1552_stage_own_dup_still_rides_the_pipe() {
    // The stage's OWN `2>&1` resolves against its pipe-bound fd 1
    // (GNU: the diagnostic becomes the next stage's stdin and is
    // never displayed); only the second stage's diagnostic, on the
    // inherited stderr, is visible.
    let (stdout, stderr, code) =
        rubash_c("PATH=/nonexistent; nosuch1552a 2>&1 | nosuch1552b; echo rc=$?");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("rc=127"), "stdout: {stdout}");
    assert!(
        !stdout.contains("nosuch1552a: command not found"),
        "stdout: {stdout}"
    );
    assert!(
        stderr.contains("nosuch1552b: command not found"),
        "stderr: {stderr}"
    );
}
