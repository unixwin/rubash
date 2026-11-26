//! rubash#1399 — `kill -l -s` (option missing its argument) was
//! diagnosed "invalid signal"; GNU reports
//! "kill: -s: option requires an argument" (same for -n), rc=1.
//! `kill -l -s TERM` lists the signal table with rc=0.
//!
//! Probe: clusters/z3/probes/t1399.sh.
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

#[test]
fn kill_missing_option_argument_diagnostic() {
    let (stdout, stderr, code) = rubash(
        "kill -l -s; echo \"rc=$?\"\n\
         kill -l -n; echo \"rc=$?\"\n\
         kill -s; echo \"rc=$?\"\n\
         kill -l -s TERM; echo \"rc=$?\"",
    );
    assert!(stdout.starts_with("rc=1\nrc=1\nrc=1\n"), "stdout: {stdout}");
    assert!(stdout.contains("1) SIGHUP"), "stdout: {stdout}");
    assert!(stdout.contains("9) SIGKILL"), "stdout: {stdout}");
    assert!(stdout.ends_with("rc=0\n"), "stdout: {stdout}");
    assert_eq!(
        stderr.matches("option requires an argument").count(),
        3,
        "stderr: {stderr}"
    );
    assert!(stderr.contains("kill: -s:"), "stderr: {stderr}");
    assert!(stderr.contains("kill: -n:"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}
