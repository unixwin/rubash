//! rubash#2488 — `wait <pid>` on a stopped job printed
//! `warning: wait_for_job: job 1 is stopped`; GNU prints nothing there.
//!
//! GNU's gate is lexical on the operand form: builtins/wait.def:316
//! sends `%...` jobspec operands to wait_for_job (jobs.c:3409-3410
//! warns when the job is stopped), while a numeric pid goes to
//! wait_for_single_pid (wait.def:277; jobs.c:2729), which returns the
//! stop status (128+SIGTSTP = 147) silently. Expectations verified
//! against GNU bash 5.3.20 in `-c` form (the r46 jr09 shape); the
//! `bash: line 1:` diagnostic prefix is rubash's established -c
//! rendering, byte-identical to GNU invoked under the name `bash`.

#![cfg(unix)]

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

const POLL: &str = "for i in 1 2 3 4 5 6 7 8 9 10; do if jobs -s | grep -q Stopped; then break; fi; sleep 0.2; done";

/// jr09 verbatim: the pid form must be silent, statuses unchanged.
#[test]
fn wait_pid_on_stopped_job_is_silent() {
    let script = format!(
        "set -m; sleep 2 & pid=$!; kill -STOP $pid; {POLL}; wait $pid; echo waitrc=$?; kill $pid; kill -CONT $pid; sleep 0.3; wait $pid; echo killrc=$?"
    );
    let (stdout, stderr, code) = rubash(&script);
    assert_eq!(stdout, "waitrc=147\nkillrc=143\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// The jobspec form keeps GNU's warning (wait.def:316 -> jobs.c:3410).
#[test]
fn wait_jobspec_on_stopped_job_warns() {
    let script = format!(
        "set -m; sleep 2 & pid=$!; kill -STOP $pid; {POLL}; wait %1; echo specrc=$?; kill $pid; kill -CONT $pid; sleep 0.3; wait $pid; echo killrc=$?"
    );
    let (stdout, stderr, code) = rubash(&script);
    assert_eq!(stdout, "specrc=147\nkillrc=143\n");
    assert_eq!(
        stderr,
        "bash: line 1: warning: wait_for_job: job 1 is stopped\n"
    );
    assert_eq!(code, Some(0));
}

/// Multi-operand: the gate applies per operand, in either order —
/// exactly one warning line, from the `%` operand alone.
#[test]
fn wait_mixed_operands_warn_only_for_jobspec() {
    for operands in ["$pid %1", "%1 $pid"] {
        let script = format!(
            "set -m; sleep 2 & pid=$!; kill -STOP $pid; {POLL}; wait {operands}; echo rc=$?; kill $pid; kill -CONT $pid; sleep 0.3; wait $pid; echo killrc=$?"
        );
        let (stdout, stderr, code) = rubash(&script);
        assert_eq!(stdout, "rc=147\nkillrc=143\n", "operands: {operands}");
        assert_eq!(
            stderr,
            "bash: line 1: warning: wait_for_job: job 1 is stopped\n",
            "operands: {operands}"
        );
        assert_eq!(code, Some(0), "operands: {operands}");
    }
}
