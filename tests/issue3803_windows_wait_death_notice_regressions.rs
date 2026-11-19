//! rubash#3803: on Windows, an immediate `wait` after a job's death
//! prints no signal-death notice.
//!
//! GNU jobs.c:3385 `wait_for` calls `notify_and_cleanup(job)` — the
//! `bash: line 1: PID Killed <cmd>` stderr notice prints inside `wait`
//! as soon as the death is noticed. On Windows the wait's own reap
//! never queued the notice: `exit_status_signal` is always None there
//! (no WTERMSIG), so `wait_for_background_pid`'s poll loop skipped
//! `note_child_signal_death` and `execute_wait` drained nothing. The
//! boundary sweep usually loses the immediate-wait race (its `try_wait`
//! runs microseconds after the kill), so the poll loop is the reap
//! that matters — and it was blind. The fix decodes the 128+signal
//! exit code rubash's own kills already encode
//! (builtins/kill.rs `signal_process` → TerminateProcess(128+signal)),
//! the same convention the boundary reap already uses.
//!
//! Each check runs in a FRESH rubash.exe process. `exec 2>&1` merges
//! the shell's stderr into the captured stream so the notice is
//! observable in true output order.

#![cfg(windows)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_rubash(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3803-{}", std::process::id()));
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

#[test]
fn k9_sleep_then_immediate_wait_prints_notice() {
    let out = run_merged("sleep 30 & p=$!; kill -9 $p; wait $p; echo wrc=$?");
    assert!(out.contains("Killed"), "notice missing in: {out}");
    assert!(out.contains("wrc=137"), "wrc=137 missing in: {out}");
}

#[test]
fn k9_sleep_jobspec_immediate_wait_prints_notice() {
    // The issue's exact seed form: jobspec kill + jobspec wait.
    let out = run_merged("sleep 30 & kill -9 %1; wait %1; echo wrc=$?");
    assert!(out.contains("Killed"), "notice missing in: {out}");
    assert!(out.contains("wrc=137"), "wrc=137 missing in: {out}");
}

#[test]
fn k9_sleep_bare_wait_prints_notice() {
    let out = run_merged("sleep 30 & p=$!; kill -9 $p; wait; echo done");
    assert!(out.contains("Killed"), "notice missing in: {out}");
    assert!(out.contains("done"), "done missing in: {out}");
}

#[test]
fn term_death_prints_no_notice() {
    // GNU prints no notice for a SIGTERM background death
    // (jobs.c:4625 DONT_REPORT_SIGTERM); the wait must still report
    // the 143 status.
    let out = run_merged("sleep 30 & p=$!; kill -TERM $p; wait $p; echo wrc=$?");
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
