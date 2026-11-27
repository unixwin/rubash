//! Regression pins for rubash#1048 and rubash#1256 (job table bookkeeping
//! and async job state-change notices).
//!
//! #1048: `disown -h %1` REMOVED the job from the table instead of marking
//! it (SIGHUP immunity). GNU bash 5.3 (jobs.c `disown`, `-h` marks each job
//! so no SIGHUP is sent on shell exit): the job stays listed and
//! `wait %1` still reaps it with rc 0. Plain `disown %1` removes on both
//! shells — only the `-h` leg diverged. Issue repro:
//!   sleep 5 & disown -h %1; jobs; wait %1; echo rc=$?
//!   GNU: job still listed, wait rc=0.  Old niu: jobs empty, wait rc=127.
//!
//! #1256: async job state-change notices were missing entirely. GNU
//! (jobs.c notify / the `jobs` builtin's listing of not-yet-notified jobs)
//! shows a terminated background job in the next `jobs` listing as
//! `[1]+ Terminated  sleep 5` and a completed one as `[1]+ Done  sleep 0.2`
//! — also non-interactively, because the job stays in the table until a
//! reader command observes it. (The `[1] PID` start notice is
//! interactive-only; the Terminated/Done listing half is not.) Issue repro:
//!   sleep 30 & kill %1; sleep 1; jobs
//!   GNU: `[1]+  Terminated  sleep 30`.  Old niu: (nothing).
//!
//! Expected outputs below were byte-checked against GNU bash 5.3.20
//! (self-built oracle) on Linux x86_64: `jobs` after kill prints
//! `[1]+  Terminated                 sleep 5`; after natural completion
//! `[1]+  Done                       sleep 0.2`. Assertions use the status
//! word + command text, not the column padding.

use std::path::PathBuf;
use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Run `script` in a fresh scratch dir so `jobs > file` captures can be
/// read back without racy stdout interleaving.
fn rubash_in_dir(script: &str, tag: &str) -> (PathBuf, String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-bfd-jobs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(&dir)
        .output()
        .expect("spawn rubash");
    (
        dir.clone(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn read(dir: &PathBuf, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

#[test]
fn issue1048_disown_h_marks_job_not_removes() {
    let (dir, _out, _err, _code) = rubash_in_dir(
        "sleep 0.5 & disown -h %1\njobs > jobs.out\nwait %1; echo rc=$? > wait.out",
        "1048h",
    );
    let jobs = read(&dir, "jobs.out");
    let wait = read(&dir, "wait.out");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        jobs.contains("sleep 0.5"),
        "disown -h must keep the job listed, jobs.out was {jobs:?}"
    );
    assert_eq!(
        wait.trim(),
        "rc=0",
        "wait %1 after disown -h must reap with rc 0, got {wait:?}"
    );
}

#[test]
fn issue1048_plain_disown_removes_job_control_leg() {
    // The non-divergent half of #1048, pinned so a fix for -h cannot
    // accidentally make plain disown keep the job.
    let (dir, _out, _err, _code) = rubash_in_dir(
        "sleep 0.5 & disown %1\njobs > jobs.out\nwait %1 2>/dev/null; echo rc=$? > wait.out",
        "1048plain",
    );
    let jobs = read(&dir, "jobs.out");
    let wait = read(&dir, "wait.out");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !jobs.contains("sleep"),
        "plain disown must remove the job from the table, jobs.out was {jobs:?}"
    );
    assert_eq!(
        wait.trim(),
        "rc=127",
        "wait %1 after plain disown reports no such job (127), got {wait:?}"
    );
}

#[test]
fn issue1256_terminated_notice_in_jobs_listing() {
    let (stdout, _stderr, _code) = rubash("sleep 5 & kill %1; sleep 0.5; jobs");
    assert!(
        stdout.contains("Terminated") && stdout.contains("sleep 5"),
        "jobs must surface the killed job as Terminated, stdout was {stdout:?}"
    );
}

#[test]
fn issue1256_done_notice_in_jobs_listing() {
    let (stdout, _stderr, _code) = rubash("sleep 0.2 & sleep 0.6; jobs");
    assert!(
        stdout.contains("Done") && stdout.contains("sleep 0.2"),
        "jobs must surface the completed job as Done, stdout was {stdout:?}"
    );
}

#[test]
fn issue1256_interactive_start_notice_prints_job_and_pid() {
    // The interactive half of #1256: starting a background job prints
    // `[1] <pid>` to stderr (GNU jobs.c job start notification). The
    // engine applies interactive defaults under -i (see the #1252
    // pins), so the notice is observable without a PTY.
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-i", "-c", "sleep 0.5 & fg"])
        .output()
        .expect("spawn rubash");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let notice = stderr.lines().find(|l| l.starts_with("[1]"));
    assert!(
        matches!(notice, Some(l) if l[3..].trim().chars().all(|c| c.is_ascii_digit()) && !l[3..].trim().is_empty()),
        "interactive job start must print `[1] <pid>` on stderr, got: {stderr:?}"
    );
}
