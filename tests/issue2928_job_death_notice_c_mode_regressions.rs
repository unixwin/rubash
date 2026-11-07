//! rubash#2928: the Killed job-death notice is printed on async reap
//! in `-c` mode; GNU prints it only at `wait` (or at the next input
//! read).
//!
//! GNU ground truth (bash 5.3.20): the notice fires when the reader
//! pulls the next physical input line — parse.y:2524-2533 `shell_getc`
//! calls `notify_and_cleanup(-1)` after `line_number++`, in
//! non-interactive mode unconditionally. A single-list `-c` string
//! gets no second pull (builtins/evalstring.c `parse_and_execute`
//! checks string exhaustion before the next parse), so no notice;
//! a `-c` string with a second line notifies with that line's number
//! (`line 2`); a script file notifies at the first read after the
//! death is recorded — `line 2` (the EOF pull) for a 1-line file,
//! `line 3` for the 3-line seed (the death records during line 2's
//! `sleep 0.5` foreground wait), `line 4` for the 4-line seed (the
//! kill alone on line 2, recorded during line 3's sleep). Rubash
//! flushed the notice at every top-level command boundary
//! (ast_exec.rs:952) and stamped the previous command's line — wrong
//! on WHEN and on the STAMP.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_shell(body: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_script_file(body: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-2928-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("seed.sh");
    {
        let mut file = std::fs::File::create(&path).expect("write script");
        file.write_all(body.as_bytes()).expect("write script");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue seed: `-c`, one list, async KILL — GNU stderr is empty.
#[test]
fn c_mode_single_list_async_kill_prints_no_notice() {
    let (stdout, stderr, rc) = run_shell("sleep 3 & p=$!; kill -KILL $p; sleep 0.5; echo done");
    assert_eq!(rc, Some(0));
    assert!(
        stdout.contains("done"),
        "seed must run to completion: {stdout:?}"
    );
    assert!(
        stderr.is_empty(),
        "GNU prints no notice for a single-list -c run: {stderr:?}"
    );
}

/// `-c` with a second line: the notice fires at that read, stamped
/// `line 2`.
#[test]
fn c_mode_second_line_notifies_with_line_2() {
    let (stdout, stderr, rc) = run_shell("sleep 3 & p=$!; kill -KILL $p; sleep 0.5\necho done");
    assert_eq!(rc, Some(0));
    assert!(stdout.contains("done"), "seed must complete: {stdout:?}");
    assert!(
        stderr.contains("line 2:"),
        "notice should be stamped with the line being read (line 2): {stderr:?}"
    );
    assert!(
        stderr.contains("Killed"),
        "notice should name the signal: {stderr:?}"
    );
}

/// Script file, one line: the notice fires at the EOF pull, stamped
/// `line 2`.
#[test]
fn script_file_single_line_notifies_with_line_2() {
    let (stdout, stderr, rc) =
        run_script_file("sleep 3 & p=$!; kill -KILL $p; sleep 0.5; echo done\n");
    assert_eq!(rc, Some(0));
    assert!(stdout.contains("done"), "seed must complete: {stdout:?}");
    assert!(
        stderr.contains("line 2:"),
        "notice should be stamped line 2 (the EOF pull): {stderr:?}"
    );
}

/// Script file, three lines: the death is recorded during line 2's
/// `sleep 0.5` foreground wait, so the first read after the recording
/// is line 3 (`echo done`) and the notice is stamped `line 3` — not
/// the EOF pull. (The fail-first draft of this test expected `line 4`
/// by extrapolating the 4-line shape below; GNU stamps `line 3` here,
/// 3/3 runs, and rubash matches — the expectation was a test error,
/// adjudicated against the oracle 2026-11-07.)
#[test]
fn script_file_multi_line_notifies_with_line_3() {
    let (stdout, stderr, rc) =
        run_script_file("sleep 3 & p=$!\nkill -KILL $p; sleep 0.5\necho done\n");
    assert_eq!(rc, Some(0));
    assert!(stdout.contains("done"), "seed must complete: {stdout:?}");
    assert!(
        stderr.contains("line 3:"),
        "notice should be stamped line 3 (the first read after the death is recorded): {stderr:?}"
    );
}

/// Script file, four lines: the kill is alone on line 2, so the death
/// is only recorded during line 3's `sleep 0.5` — the first read after
/// the recording is line 4 (`echo done`), and the notice is stamped
/// `line 4`. This is the shape that distinguishes the read-time
/// stamp from the just-finished line.
#[test]
fn script_file_death_recorded_mid_stream_notifies_at_next_read() {
    let (stdout, stderr, rc) =
        run_script_file("sleep 3 & p=$!\nkill -KILL $p\nsleep 0.5\necho done\n");
    assert_eq!(rc, Some(0));
    assert!(stdout.contains("done"), "seed must complete: {stdout:?}");
    assert!(
        stderr.contains("line 4:"),
        "notice should be stamped line 4 (the first read after the death is recorded): {stderr:?}"
    );
    assert!(
        stderr.contains("Killed"),
        "notice should name the signal: {stderr:?}"
    );
}

/// Positive control: a stopped job that is then KILLed and waited on
/// prints its notice once, at the wait, in both modes.
#[test]
fn stopped_then_killed_job_notifies_once_at_wait() {
    let (stdout, stderr, rc) = run_shell(
        "sleep 3 & p=$!; kill -STOP $p; sleep 0.2; kill -KILL $p; \
         wait $p; echo w=$?",
    );
    assert_eq!(rc, Some(0));
    assert!(
        stdout.contains("w=137"),
        "wait should report 137: {stdout:?}"
    );
    assert_eq!(
        stderr.matches("Killed").count(),
        1,
        "exactly one Killed notice, at the wait: {stderr:?}"
    );
}
