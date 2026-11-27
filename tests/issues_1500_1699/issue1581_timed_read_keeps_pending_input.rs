//! rubash#1581: when a pipeline consumer's `read -t` times out on a
//! shell-form (materialized) producer's stream, the input that had
//! not yet arrived must stay pending for the consumer's NEXT read —
//! GNU read.def assigns the in-time prefix, returns 128+SIGALRM, and
//! the fd cursor keeps the rest of the stream. Rubash dropped the
//! not-yet-arrived record: the following untimed read saw EOF.
//!
//! Oracle: GNU bash 5.3.20, script files.

#![cfg(unix)]

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// Run a script file with a hard wall-clock deadline (the cases use
/// 1s producers; a hang must fail the test, not the suite).
fn run_script(script: &str) -> String {
    let dir = scratch_tempdir();
    let script_path = dir.path().join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let mut child = Command::new(rubash_bin())
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", dir.path())
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let mut stdout_pipe = child.stdout.take().expect("stdout pipe");
    let drain = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                assert!(Instant::now() < deadline, "script timed out: {script}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    let bytes = drain.join().expect("drain");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn issue1581_timed_out_read_leaves_late_record_for_next_read() {
    // The issue's regression seed.
    let stdout = run_script(
        "{ sleep 1; echo A; } | { read -t 0.3 x; echo early_rc=$?; read y; echo got=$y; }\n",
    );
    assert_eq!(stdout, "early_rc=142\ngot=A\n");
}

#[test]
fn issue1581_read_after_served_timed_read_keeps_rest_of_stream() {
    let stdout = run_script(
        "{ echo B; sleep 1; echo A; } | { read -t 0.3 x; read y; echo \"x=$x y=$y\"; }\n",
    );
    assert_eq!(stdout, "x=B y=A\n");
}

#[test]
fn issue1581_read_after_timed_read_with_instant_producer() {
    let stdout =
        run_script("{ echo B; echo A; } | { read -t 0.3 x; read y; echo \"x=$x y=$y\"; }\n");
    assert_eq!(stdout, "x=B y=A\n");
}

#[test]
fn issue1581_second_timed_read_with_generous_deadline_serves() {
    let stdout =
        run_script("{ sleep 1; echo A; } | { read -t 0.3 x; read -t 5 y; echo \"y=$y rc=$?\"; }\n");
    assert_eq!(stdout, "y=A rc=0\n");
}

#[test]
fn issue1581_mapfile_after_timed_out_read_sees_pending_records() {
    let stdout = run_script(
        "{ sleep 1; echo A; echo B; } | { read -t 0.3 x; echo early_rc=$?; mapfile -t arr; echo \"n=${#arr[@]} a=${arr[*]}\"; }\n",
    );
    assert_eq!(stdout, "early_rc=142\nn=2 a=A B\n");
}

#[test]
fn issue1581_external_followup_after_timed_out_read_sees_pending_data() {
    // The discard was not specific to a second `read`: any followup
    // command consuming the stream (here external `cat`) must receive
    // the not-yet-arrived bytes once they land.
    let stdout = run_script("{ sleep 1; echo A; } | { read -t 0.3 x; echo early_rc=$?; cat; }\n");
    assert_eq!(stdout, "early_rc=142\nA\n");
}

#[test]
fn issue1581_untimed_reads_on_timed_stream_are_unaffected() {
    // Control: without an armed deadline anywhere, sequential reads
    // over the same materialized stream work (already true before).
    let stdout =
        run_script("{ echo B; sleep 1; echo A; } | { read x; read y; echo \"x=$x y=$y\"; }\n");
    assert_eq!(stdout, "x=B y=A\n");
}
