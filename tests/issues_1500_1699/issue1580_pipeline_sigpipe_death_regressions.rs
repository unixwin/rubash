//! Issue rubash#1580: all-external pipeline members did not die of
//! SIGPIPE. The pipeline spawners built their members through
//! `external_command_for_named_program` but never installed the child
//! signal reset, so members inherited the SIGPIPE mask block the
//! executor holds around pipeline execution (rubash#1333's guard) —
//! the kernel could never deliver SIGPIPE, writes surfaced as EPIPE,
//! members exited 1 with their own "Broken pipe" diagnostics leaked
//! to the shell's stderr, and PIPESTATUS recorded 1 instead of 141.
//!
//! GNU model: execute_cmd.c:2620 forks every pipeline element;
//! execute_cmd.c:4224 runs trap.c:1489 restore_original_signals in
//! the child between fork and exec, so each member is born with the
//! entry dispositions and an unblocked mask. A member writing to a
//! pipe whose reader has closed dies of SIGPIPE; jobs.c:2958
//! process_exit_status surfaces 128 + 13 = 141 and the member's
//! stderr stays silent (it died mid-write, before any diagnostic).
//!
//! Expected outputs verified against GNU bash 5.3.20 (script files).
//! Unix-only: SIGPIPE death is the contract under test; the Windows
//! lingerer synthesis is rubash#382's surface.

#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUN_LIMIT: Duration = Duration::from_secs(30);

struct RunOutcome {
    timed_out: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(tag: &str, script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue1580-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1580 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("BASH_ENV")
        .spawn()
        .expect("spawn rubash");

    let stdout_pipe = child.stdout.take().expect("piped stdout");
    let stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stdout_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = stderr_pipe;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        buffer
    });

    let start = Instant::now();
    let (timed_out, code) = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break (false, status.code()),
            None => {
                if start.elapsed() > RUN_LIMIT {
                    let _ = child.kill();
                    let _ = child.wait();
                    break (true, None);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    RunOutcome {
        timed_out,
        code,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    }
}

#[test]
fn issue1580_three_stage_members_die_of_sigpipe() {
    // Issue seed: both upstream members die of SIGPIPE (141), the
    // consumer exits 0, and no Broken pipe diagnostic leaks.
    let out = run_rubash_script(
        "three",
        "yes | cat | head -1\necho \"PIPESTATUS=${PIPESTATUS[*]}\"\n",
    );
    assert!(!out.timed_out, "pipeline hung");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "y\nPIPESTATUS=141 141 0\n");
    assert_eq!(out.stderr, "", "stderr: {}", out.stderr);
}

#[test]
fn issue1580_two_stage_producer_dies_of_sigpipe() {
    let out = run_rubash_script(
        "two",
        "yes | head -1\necho \"PIPESTATUS=${PIPESTATUS[*]}\"\n",
    );
    assert!(!out.timed_out, "pipeline hung");
    assert_eq!(out.stdout, "y\nPIPESTATUS=141 0\n");
    assert_eq!(out.stderr, "", "stderr: {}", out.stderr);
}

#[test]
fn issue1580_pipefail_propagates_sigpipe_status() {
    let out = run_rubash_script(
        "pipefail",
        "set -o pipefail\nyes | cat | head -1\necho \"rc=$?\"\n",
    );
    assert!(!out.timed_out, "pipeline hung");
    assert_eq!(out.stdout, "y\nrc=141\n");
    assert_eq!(out.stderr, "", "stderr: {}", out.stderr);
}

#[test]
fn issue1580_no_false_sigpipe_for_well_behaved_members() {
    // Members whose consumer drains everything exit normally: the
    // reset must not turn ordinary exits into signal deaths.
    let out = run_rubash_script(
        "wellbehaved",
        "printf 'a\\nb\\n' | cat | head -2\necho \"PIPESTATUS=${PIPESTATUS[*]}\"\n",
    );
    assert!(!out.timed_out, "pipeline hung");
    assert_eq!(out.stdout, "a\nb\nPIPESTATUS=0 0 0\n");
    assert_eq!(out.stderr, "", "stderr: {}", out.stderr);
}
