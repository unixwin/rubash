//! rubash#1302: ambiguous `%?pat` jobspecs must error, not silently resolve.
//!
//! GNU jobs.c job lookup (verified byte-identical vs GNU bash 5.3.20,
//! Termux, script-file probes 2026-10-23): `%?pat` is a case-folded
//! substring match (`strcasestr`), `%name` a prefix (`STREQN`), `%+x`/
//! `%-x`/`%%` first-byte dispatch. Multiple matches are ambiguous and each
//! consumer reports per GNU: kill (diagnostic only, rc=1), wait/wait -n
//! (diagnostic + `no such job`, rc=127), jobs/disown (diagnostic +
//! `no such job`, rc=1), fg/bg (diagnostic only — unreachable without job
//! control in scripts), jobs -x (diagnostic + passthrough, rc=0).
//! A single match resolves silently with rc=0; `%1`/`%+`/`%-`/`%%`
//! dispatch is unaffected. POSIX `kill` keeps rc=0 with the diagnostic.

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;

fn run_held(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("__RUBASH_TRAP_ORIG_IGN")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let mut stdin = child.stdin.take().expect("piped stdin");
    let keeper = thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(700));
        drop(stdin.write_all(b""));
        drop(stdin);
    });
    let output = child.wait_with_output().expect("wait rubash");
    let _ = keeper.join();
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Two live jobs sharing the `sleep` command text; the spacer keeps both
/// in the table deterministically. Cleans up so no sleep outlives the test.
fn two_job_env() -> (&'static str, &'static str) {
    (
        "sleep 50 &\nP1=$!\nsleep 1\nsleep 51 &\nP2=$!\nsleep 1\n",
        "\nkill $P1 $P2 2>/dev/null; wait 2>/dev/null",
    )
}

fn dup_case(body: &str) -> String {
    let (setup, teardown) = two_job_env();
    format!("{setup}{body}; echo \"rc=$?\"{teardown}\n")
}

#[test]
fn kill_ambiguous_reports_and_returns_one() {
    let (stdout, stderr, rc) = run_held(&dup_case("kill -0 %?sleep"));
    assert_eq!(stdout, "rc=1\n");
    // The diagnostic echoes the resolved command name (sigil stripped).
    assert!(
        stderr.contains("kill: sleep: ambiguous job spec"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn wait_ambiguous_returns_127() {
    let (stdout, stderr, rc) = run_held(&dup_case("wait %?sleep"));
    assert_eq!(stdout, "rc=127\n");
    assert!(
        stderr.contains("wait: sleep: ambiguous job spec"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn jobs_ambiguous_reports_both_lines_returns_one() {
    let (stdout, stderr, rc) = run_held(&dup_case("jobs %?sleep"));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("jobs: sleep: ambiguous job spec"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("jobs: %?sleep: no such job"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn disown_ambiguous_reports_both_lines_returns_one() {
    let (stdout, stderr, rc) = run_held(&dup_case("disown %?sleep"));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("disown: sleep: ambiguous job spec"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("disown: %?sleep: no such job"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn jobs_dash_x_passes_pattern_through() {
    let (stdout, stderr, rc) = run_held(&dup_case("jobs -x echo hi"));
    assert!(stdout.contains("hi\n"), "stdout: {stdout:?}");
    assert!(stdout.contains("rc=0"), "stdout: {stdout:?}");
    assert!(!stderr.contains("ambiguous"), "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

#[test]
fn wait_n_ambiguous_returns_127() {
    let (stdout, stderr, rc) = run_held(&dup_case("wait -n %?sleep"));
    assert_eq!(stdout, "rc=127\n");
    assert!(
        stderr.contains("wait: sleep: ambiguous job spec"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("wait: %?sleep: no such job"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn fg_bg_report_no_job_control_in_scripts() {
    // Below the jobspec layer: scripts have no job control, so fg/bg
    // never reach resolution (GNU identical).
    for cmd in ["fg %?sleep", "bg %?sleep"] {
        let (stdout, stderr, rc) = run_held(&dup_case(cmd));
        assert_eq!(stdout, "rc=1\n", "for {cmd}");
        let builtin = cmd.split(' ').next().unwrap();
        assert!(
            stderr.contains(&format!("{builtin}: no job control")),
            "stderr: {stderr:?}"
        );
        assert_eq!(rc, Some(0));
    }
}

#[test]
fn prefix_match_reports_typed_text() {
    let (stdout, stderr, rc) = run_held(&dup_case("kill -0 %sl"));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("kill: sl: ambiguous job spec"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn question_pattern_match_is_case_folded() {
    let (stdout, stderr, rc) = run_held(&dup_case("kill -0 %?SLEEP"));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("kill: SLEEP: ambiguous job spec"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn single_pattern_match_resolves_silently() {
    let (stdout, stderr, rc) = run_held(&dup_case("kill -0 %?51"));
    assert_eq!(stdout, "rc=0\n");
    assert!(!stderr.contains("ambiguous"), "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

#[test]
fn exact_current_previous_dispatch_unaffected() {
    for spec in ["%1", "%+", "%-", "%%"] {
        let (stdout, stderr, rc) = run_held(&dup_case(&format!("kill -0 {spec}")));
        assert_eq!(stdout, "rc=0\n", "for {spec}");
        assert!(
            !stderr.contains("ambiguous"),
            "stderr: {stderr:?} for {spec}"
        );
        assert_eq!(rc, Some(0));
    }
}

#[test]
fn posix_kill_ambiguous_keeps_rc_zero_with_diagnostic() {
    let (setup, teardown) = two_job_env();
    let script = format!("{setup}set -o posix\nkill -0 %?sleep; echo \"rc=$?\"{teardown}\n");
    let (stdout, stderr, rc) = run_held(&script);
    assert_eq!(stdout, "rc=0\n");
    assert!(
        stderr.contains("kill: sleep: ambiguous job spec"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn empty_table_percent_spec_warns_and_fails() {
    let (stdout, stderr, rc) = run_held("jobs %5; echo rc=$?\n");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("jobs: %5: no such job"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}
