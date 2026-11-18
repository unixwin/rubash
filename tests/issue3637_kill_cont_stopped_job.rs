//! `kill -s CONT` on a job already recorded Stopped emulates `bg`:
//! GNU jobs.c:4013-4018 kill_pid publishes the Running transition at
//! DELIVERY time (`set_job_running` + J_NOTIFIED), not at the next
//! notification point. Before this fix rubash deferred the mark, so
//! the immediate `jobs -r`/`jobs -s` pair after the CONT still listed
//! the job under stopped jobs — the jobs.tests residual in the
//! run-jobs lane (#2666).
//!
//! Oracle (GNU bash 5.3.20, `set -m`):
//!   sleep 300 &  sleep 350 &  sleep 400 &
//!   kill -STOP %2 ; sleep 1
//!   jobs -s            -> [2]+  Stopped   sleep 350
//!   kill -s CONT %2
//!   jobs -r            -> [2]+  Running   sleep 350 &  (+ 300/400)
//!   jobs -s            -> (empty)

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

#[test]
fn kill_cont_on_recorded_stopped_job_publishes_running_immediately() {
    let script = "set -m\n\
                  sleep 300 &\n\
                  sleep 350 &\n\
                  sleep 400 &\n\
                  kill -STOP %2\n\
                  sleep 1\n\
                  echo stopped1:\n\
                  jobs -s\n\
                  kill -s CONT %2\n\
                  echo running:\n\
                  jobs -r\n\
                  echo stopped2:\n\
                  jobs -s\n\
                  kill -n 9 %1 %2 %3 2>/dev/null\n\
                  wait 2>/dev/null\n\
                  echo done\n";
    let (stdout, stderr, rc) = run_rubash(script);
    // GNU 5.3.20 renders the same script byte-for-byte modulo pids.
    let expected = "stopped1:\n\
                    [2]+  Stopped                    sleep 350\n\
                    running:\n\
                    [1]   Running                    sleep 300 &\n\
                    [2]+  Running                    sleep 350 &\n\
                    [3]-  Running                    sleep 400 &\n\
                    stopped2:\n\
                    done\n";
    assert_eq!(stdout, expected);
    assert_eq!(rc, Some(0));
    // stderr may carry the async `Killed` notice; only the state
    // listings are pinned.
    let _ = stderr;
}
