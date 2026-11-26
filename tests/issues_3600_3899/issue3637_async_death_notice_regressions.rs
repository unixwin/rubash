//! rubash#3637: async-job signal-death notice placement.
//!
//! GNU jobs.c:3385 wait_for calls notify_and_cleanup(job) — the
//! `bash: line 1: PID Killed <cmd>` stderr notice prints inside `wait`,
//! exactly once, as soon as the death is noticed. If `jobs` displays
//! the death first, the job is J_NOTIFIED (jobs.c:4570
//! notify_of_job_status skips notified jobs) and no stderr notice
//! prints at all — exactly one `Killed` line, the jobs-table line.
//!
//! Seed: r118 verdict (seed_r118a.sh), GNU 9/9, rubash prescreen 4/9
//! (checks 1-5 fail). The pin adds one order cell (check 8): the
//! notice must land DURING `wait` (after the pre-wait marker, before
//! the post-wait marker) — a stream-end print would fail it.
//!
//! Each check runs in a FRESH shell process. `exec 2>&1` merges the
//! shell's stderr into the captured stream so the notice is observable
//! in true output order.

#![cfg(unix)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_rubash(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3637-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let script_path = dir.join("script.sh");
    std::fs::write(&script_path, script).expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.current_dir(&dir)
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait().expect("poll rubash") {
            Some(_) => break,
            None => {
                assert!(Instant::now() < deadline, "rubash timed out: {script}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    let output = child.wait_with_output().expect("collect rubash output");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
    )
}

/// Run `body` as a `-c`-style script file with stderr merged into
/// stdout (true order). Returns the merged output.
fn run_merged(body: &str) -> String {
    let script = format!("exec 2>&1\n{body}\n");
    let (stdout, stderr, _rc) = run_rubash(&script);
    assert!(
        stderr.is_empty(),
        "stderr must be empty after exec 2>&1, got: {stderr}"
    );
    stdout
}

fn count_killed(out: &str) -> usize {
    out.matches("Killed").count()
}

#[test]
fn k9_sleep_then_wait_prints_notice_once() {
    let out = run_merged(
        "sleep 30 & p=$!; kill -9 $p; sleep 0.2; kill -0 $p 2>&1; echo rc=$?; wait $p; echo wrc=$?",
    );
    assert!(out.contains("Killed"), "notice missing in: {out}");
    assert!(out.contains("wrc=137"), "wrc=137 missing in: {out}");
}

#[test]
fn k9_sleep_jobs_then_wait_prints_exactly_one_killed_line() {
    let out = run_merged("sleep 30 & p=$!; kill -9 $p; sleep 0.2; jobs; wait $p; echo wrc=$?");
    assert_eq!(
        count_killed(&out),
        1,
        "want exactly one Killed line (the jobs-table line), got {} in: {out}",
        count_killed(&out)
    );
    assert!(out.contains("wrc=137"), "wrc=137 missing in: {out}");
}

#[test]
fn kill_sleep_jobs_echo_then_wait_prints_exactly_one_killed_line() {
    let out = run_merged(
        "sleep 30 & p=$!; kill -KILL $p; sleep 0.2; jobs; echo done; wait $p; echo wrc=$?",
    );
    assert_eq!(
        count_killed(&out),
        1,
        "want exactly one Killed line (the jobs-table line), got {} in: {out}",
        count_killed(&out)
    );
    assert!(out.contains("wrc=137"), "wrc=137 missing in: {out}");
}

#[test]
fn k9_setm_sleep_then_wait_prints_notice() {
    let out =
        run_merged("set -m; sleep 30 & p=$!; kill -9 $p; sleep 0.2; echo hi; wait $p; echo wrc=$?");
    assert!(out.contains("Killed"), "notice missing in: {out}");
    assert!(out.contains("wrc=137"), "wrc=137 missing in: {out}");
}

#[test]
fn k9_sleep_then_wait_prints_notice_rerun() {
    let out = run_merged(
        "sleep 30 & p=$!; kill -9 $p; sleep 0.2; kill -0 $p 2>&1; echo rc=$?; wait $p; echo wrc=$?",
    );
    assert!(out.contains("Killed"), "notice missing in: {out}");
    assert!(out.contains("wrc=137"), "wrc=137 missing in: {out}");
}

#[test]
fn control_k9_immediate_wait_prints_notice() {
    let out = run_merged("sleep 30 & p=$!; kill -9 $p; wait $p; echo wrc=$?");
    assert!(out.contains("Killed"), "notice missing in: {out}");
    assert!(out.contains("wrc=137"), "wrc=137 missing in: {out}");
}

#[test]
fn control_term_death_prints_no_notice() {
    let out = run_merged(
        "sleep 30 & p=$!; kill -TERM $p; sleep 0.2; kill -0 $p 2>&1; echo rc=$?; wait $p; echo wrc=$?",
    );
    assert!(
        !out.contains("Killed"),
        "unexpected Killed notice in: {out}"
    );
    assert!(out.contains("wrc=143"), "wrc=143 missing in: {out}");
}

#[test]
fn notice_prints_during_wait_not_at_stream_end() {
    let out =
        run_merged("sleep 30 & p=$!; kill -9 $p; sleep 0.2; echo MARKER1; wait $p; echo MARKER2");
    let killed = out.find("Killed").expect("notice missing in: {out}");
    let marker1 = out.find("MARKER1").expect("MARKER1 missing in: {out}");
    let marker2 = out.find("MARKER2").expect("MARKER2 missing in: {out}");
    assert!(
        killed > marker1 && killed < marker2,
        "notice must print during wait (between MARKER1 and MARKER2) in: {out}"
    );
}
