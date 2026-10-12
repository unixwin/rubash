//! rubash#549: jobs bad-spec leniency and `alias -p NAME`.
//!
//! Two builtin parameter-handling cells (WSL GNU 5.3.0(1) probes,
//! 2026-10-12):
//!
//! cell 1 — jobs.def's bad-spec path is lenient: an operand without the
//!   leading `%' draws `warning: N: job specification requires leading
//!   `%'` and is then resolved AS IF the `%' were there (`sleep 5 & jobs
//!   1` warns and lists job 1; `jobs 5`/`jobs 1x` warn then report
//!   `N: no such job`). Every listing failure — bare or `%N` — leaves the
//!   builtin's status at 0 (`jobs %5`, `jobs 1 %2` all return 0).
//!
//! cell 2 — GNU bash 5.3 turned `alias -p NAME` into a pure whole-table
//!   listing: the NAME operands are ignored entirely (`alias zz1=: zz2=:;
//!   alias -p zz1` prints exactly the zz1/zz2 pair, rc 0). `alias NAME`
//!   without -p still prints just the named alias.

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("__RUBASH_TRAP_ORIG_IGN")
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

/// Runs `script` with stdin held open long enough for the background job
/// to still be listed, then closes it.
fn run_with_held_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("__RUBASH_TRAP_ORIG_IGN")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    // Keep stdin open while the script runs so the job table is populated;
    // close it so the shell can reach EOF.
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

#[test]
fn bare_number_spec_warns_and_lists_the_matching_job() {
    let (stdout, stderr, rc) = run_with_held_stdin("sleep 5 & jobs 1; echo rc=$?");
    assert!(
        stdout.contains("sleep 5"),
        "the job table must still list: {stdout:?}"
    );
    assert!(
        stderr.contains("jobs: warning: 1: job specification requires leading `%'"),
        "stderr: {stderr:?}"
    );
    assert!(stdout.contains("rc=0"), "stdout: {stdout:?}");
    assert_eq!(rc, Some(0));
}

#[test]
fn bare_number_spec_without_a_match_warns_and_reports_no_such_job() {
    let (stdout, stderr, rc) = run_with_held_stdin("sleep 5 & jobs 5; echo rc=$?");
    assert_eq!(stdout, "rc=0\n");
    assert!(
        stderr.contains("jobs: warning: 5: job specification requires leading `%'"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("jobs: 5: no such job"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn non_number_bare_spec_gets_the_same_treatment() {
    let (stdout, stderr, rc) = run_with_held_stdin("sleep 5 & jobs 1x; echo rc=$?");
    assert_eq!(stdout, "rc=0\n");
    assert!(
        stderr.contains("jobs: warning: 1x: job specification requires leading `%'"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("jobs: 1x: no such job"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn percent_spec_failure_keeps_rc_zero() {
    let (stdout, stderr, rc) = run_with_held_stdin("sleep 5 & jobs %5; echo rc=$?");
    assert_eq!(stdout, "rc=0\n");
    assert!(
        stderr.contains("jobs: %5: no such job"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn mixed_spec_warning_precedes_the_listing() {
    // GNU's stderr is unbuffered and its stdout table buffered, so in a
    // merged capture the warning precedes the job line (jobs 1 probe).
    let (stdout, stderr, rc) = run_with_held_stdin("sleep 5 & jobs 1 %2; echo rc=$?");
    let merged_order = stdout.contains("sleep 5") && stdout.contains("rc=0");
    assert!(merged_order, "stdout: {stdout:?}");
    assert!(
        stderr.contains("jobs: warning: 1: job specification requires leading `%'"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("jobs: %2: no such job"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn jobs_percent_percent_still_lists_twice() {
    let (stdout, _stderr, rc) = run_with_held_stdin("sleep 5 & jobs %1 %1; echo rc=$?");
    assert_eq!(stdout.matches("sleep 5").count(), 2, "stdout: {stdout:?}");
    assert!(stdout.contains("rc=0"));
    assert_eq!(rc, Some(0));
}

#[test]
fn alias_p_with_operand_prints_the_whole_table_once() {
    let (stdout, stderr, rc) = run("alias zz1=: zz2=:; alias -p zz1; echo rc=$?");
    assert_eq!(stdout, "alias zz1=':'\nalias zz2=':'\nrc=0\n",);
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
}

#[test]
fn alias_p_with_undefined_operand_still_prints_the_table() {
    let (stdout, stderr, rc) = run("alias zz1=: zz2=:; alias -p nosuch; echo rc=$?");
    assert_eq!(stdout, "alias zz1=':'\nalias zz2=':'\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
}

#[test]
fn alias_name_without_p_still_prints_only_that_alias() {
    let (stdout, stderr, rc) = run("alias zz1=: zz2=:; alias zz1; echo rc=$?");
    assert_eq!(stdout, "alias zz1=':'\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
}

#[test]
fn alias_missing_name_without_p_reports_not_found() {
    let (stdout, stderr, rc) = run("alias nosuch; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.ends_with("alias: nosuch: not found\n"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}

#[test]
fn jobs_with_no_jobs_and_bare_number_stays_silent_rc_zero() {
    let (stdout, stderr, rc) = run("jobs 1; echo rc=$?");
    assert_eq!(stdout, "rc=0\n");
    assert!(
        stderr.contains("jobs: warning: 1: job specification requires leading `%'"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("jobs: 1: no such job"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(0));
}
