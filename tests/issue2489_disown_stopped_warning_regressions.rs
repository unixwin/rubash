//! rubash#2489 — deleting a stopped job via `disown` printed nothing;
//! GNU warns `warning: deleting stopped job %d with process group %ld`.
//!
//! GNU's gate: jobs.c:1495-1496 — delete_job warns only when the
//! caller passes DEL_WARNSTOPPED, the job is STOPPED at deletion time,
//! and subshell_environment == 0. Every disown deletion carries the
//! flag (builtins/jobs.def disown_builtin: delete_job (job, 1) per
//! spec; jobs.c:5091 delete_all_jobs for -a), while plain shell exit
//! is not a delete_job path and stays silent. Expectations verified
//! against GNU bash 5.3.20 probes (script files); the r46 jr13 shape
//! is the first test. The `bash: line 1:` prefix is rubash's
//! established -c diagnostic rendering (GNU invoked as `bash`).

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

/// The pid (== pgrp under `set -m`) a script echoes as `pid=<n>`.
fn echoed_pid(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .expect("script echoes pid=")
        .to_string()
}

/// jr13 verbatim shape: `disown %1` on a stopped job warns with the
/// job's process group; the later `wait $pid` still reads the
/// disowned-child status (0), as on GNU.
#[test]
fn disown_jobspec_of_stopped_job_warns() {
    let script = format!(
        "set -m; sleep 2 & pid=$!; echo \"pid=$pid\"; kill -STOP $pid; {POLL}; disown %1; kill -CONT $pid; sleep 0.3; wait $pid; echo waited=$?"
    );
    let (stdout, stderr, code) = rubash(&script);
    let pid = echoed_pid(&stdout);
    assert_eq!(stdout, format!("pid={pid}\nwaited=0\n"));
    assert_eq!(
        stderr,
        format!("bash: line 1: warning: deleting stopped job 1 with process group {pid}\n")
    );
    assert_eq!(code, Some(0));
}

/// The pid form of disown goes through the same delete_job call.
#[test]
fn disown_pid_of_stopped_job_warns() {
    let script = format!(
        "set -m; sleep 2 & pid=$!; echo \"pid=$pid\"; kill -STOP $pid; {POLL}; disown $pid; echo disowned; kill -CONT $pid; kill $pid; wait $pid 2>/dev/null; echo done"
    );
    let (stdout, stderr, code) = rubash(&script);
    let pid = echoed_pid(&stdout);
    assert_eq!(stdout, format!("pid={pid}\ndisowned\ndone\n"));
    assert_eq!(
        stderr,
        format!("bash: line 1: warning: deleting stopped job 1 with process group {pid}\n")
    );
    assert_eq!(code, Some(0));
}

/// Bare `disown` (current job) shares the same deletion path.
#[test]
fn disown_current_stopped_job_warns() {
    let script = format!(
        "set -m; sleep 2 & pid=$!; echo \"pid=$pid\"; kill -STOP $pid; {POLL}; disown; echo disowned; kill -CONT $pid; kill $pid; wait $pid 2>/dev/null; echo done"
    );
    let (stdout, stderr, code) = rubash(&script);
    let pid = echoed_pid(&stdout);
    assert_eq!(stdout, format!("pid={pid}\ndisowned\ndone\n"));
    assert_eq!(
        stderr,
        format!("bash: line 1: warning: deleting stopped job 1 with process group {pid}\n")
    );
    assert_eq!(code, Some(0));
}

/// `disown -a` (delete_all_jobs): one warning per stopped job, in
/// job order, each with its own process group.
#[test]
fn disown_all_warns_per_stopped_job() {
    let script = format!(
        "set -m; sleep 2 & p1=$!; sleep 2 & p2=$!; echo \"p1=$p1\"; echo \"p2=$p2\"; kill -STOP $p1; kill -STOP $p2; {POLL}; disown -a; echo cleared; kill -CONT $p1 $p2; kill $p1 $p2; wait 2>/dev/null; echo done"
    );
    let (stdout, stderr, code) = rubash(&script);
    let grab = |key: &str| {
        stdout
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .expect("script echoes pid")
            .to_string()
    };
    let (p1, p2) = (grab("p1="), grab("p2="));
    assert_eq!(stdout, format!("p1={p1}\np2={p2}\ncleared\ndone\n"));
    assert_eq!(
        stderr,
        format!(
            "bash: line 1: warning: deleting stopped job 1 with process group {p1}\nbash: line 1: warning: deleting stopped job 2 with process group {p2}\n"
        )
    );
    assert_eq!(code, Some(0));
}

/// A running job is deleted silently — the warning is stopped-only.
#[test]
fn disown_running_job_is_silent() {
    let (stdout, stderr, code) = rubash(
        "set -m; sleep 0.5 & p1=$!; disown %1; echo disowned; wait $p1; echo \"waitrc=$?\"",
    );
    assert_eq!(stdout, "disowned\nwaitrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Negative guard: a script that simply ends with a stopped job
/// still in the table produces no warning on GNU — shell exit is not
/// a delete_job path, so no warning may be added there.
#[test]
fn plain_exit_with_stopped_job_is_silent() {
    let script = format!(
        "set -m; sleep 2 & pid=$!; kill -STOP $pid; {POLL}; echo ending; kill -CONT $pid; kill $pid"
    );
    let (stdout, stderr, code) = rubash(&script);
    assert_eq!(stdout, "ending\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
