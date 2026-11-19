//! rubash#3819 — failed pipe creation under RLIMIT_NOFILE exhaustion.
//!
//! GNU bash (`execute_cmd.c:execute_pipeline`) reports `pipe error: <strerror>`
//! and exits the shell (status 129) when `pipe(2)` fails before running any
//! pipeline stage or subsequent command in the script.

#![cfg(unix)]

use std::process::Command;

fn run_rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("LC_ALL", "C")
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
fn pipe_creation_failure_under_nofile_exhaustion_exits_129() {
    let cases = [
        "ulimit -n 4; echo hi | cat; echo rc=$?",
        "ulimit -n 4; true | true; echo rc=$?",
        "ulimit -n 4; printf 'a\\nb\\n' | wc -l; echo rc=$?",
        "ulimit -n 4; /bin/echo hi | cat; echo rc=$?",
        "ulimit -n 4; sleep 0.1 | cat; echo rc=$?",
        "ulimit -n 4; echo hi | cat | cat; echo rc=$?",
        "ulimit -n 4; echo hi | cat; /bin/true; echo rc=$?",
        "ulimit -n 5; echo hi | cat | cat; echo rc=$?",
    ];
    for script in cases {
        let (stdout, stderr, code) = run_rubash(script);
        assert_eq!(stdout, "", "script {script:?} produced unexpected stdout");
        assert!(
            stderr.contains("pipe error: Too many open files"),
            "script {script:?} stderr: {stderr:?}"
        );
        assert_eq!(code, Some(129), "script {script:?} exit code");
    }
}

#[test]
fn pipe_creation_failure_in_subshell_exits_subshell_129() {
    let (stdout, stderr, code) = run_rubash("( ulimit -n 4; echo hi | cat ); echo after=$?");
    assert_eq!(stdout, "after=129\n");
    assert!(
        stderr.contains("pipe error: Too many open files"),
        "stderr: {stderr:?}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn pipelines_with_sufficient_fds_and_non_pipelines_still_succeed() {
    for (script, expected_stdout) in [
        ("ulimit -n 5; echo hi | cat; echo rc=$?", "hi\nrc=0\n"),
        ("echo hi | cat; echo rc=$?", "hi\nrc=0\n"),
        ("ulimit -n 4; echo hi; echo rc=$?", "hi\nrc=0\n"),
        ("ulimit -n 6; echo hi | cat | cat; echo rc=$?", "hi\nrc=0\n"),
    ] {
        let (stdout, stderr, code) = run_rubash(script);
        assert_eq!(stdout, expected_stdout, "script {script:?}");
        assert_eq!(stderr, "", "script {script:?}");
        assert_eq!(code, Some(0), "script {script:?}");
    }
}
