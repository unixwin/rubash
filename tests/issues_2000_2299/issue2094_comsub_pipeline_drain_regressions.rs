//! Issue rubash#2094: a pipeline stage whose output lands on a command
//! substitution's capture pipe deadlocked (or silently lost its output)
//! once the stage's stdin payload and its output each exceeded the pipe
//! buffer (~64 KiB; observed threshold between 135,000 and 140,000
//! bytes combined in flight).
//!
//! Shape: `out=$(dl | command sed '…')` where `dl` is a function whose
//! body runs `command cat <file>`. The `command sed` element dispatches
//! as a builtin stage; inside its subshell the real sed spawns through
//! the simple-command external path, which fed the child's stdin
//! payload SYNCHRONOUSLY (feed_child_stdin write_all) and only then
//! drained the shared capture pipe (ExternalStdioOutcome::CapturePipe).
//! GNU forks, so feeding and draining are concurrent by construction
//! (subst.c:7143 command_substitute dups the pipe into the child;
//! subst.c:7437 read_comsub runs before subst.c:7441 wait_for). Here
//! the child filled the capture pipe, stopped reading, the feed blocked
//! mid-payload, and the drain never started — a three-way deadlock.
//! The same serial order against a plain file endpoint lost the output
//! outright (the stage's bytes bypassed/mismatched the driver's route).
//!
//! Fix: when the stdio plan leaves a CapturePipe outcome, the drain
//! starts on its own thread BEFORE the stdin feed, so the capture is
//! emptied concurrently with the feed and the child wait — the same
//! discipline niubash#158 applied to concurrent pipeline members.
//!
//! Expected outputs are byte-verified against GNU Bash 5.3.20
//! (self-compiled oracle, script-file probes 2026-10-29). Every run is
//! deadline-bounded: a regression here HANGS, and a hanging pin must
//! fail fast instead of eating the 25-minute CI job.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PAYLOAD_LINES: usize = 3000;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2094-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Deterministic payload: 3000 lines x 50 bytes = 150,000 bytes, above
/// the observed (135000, 140000] deadlock threshold.
fn payload() -> String {
    let mut text = String::new();
    for i in 0..PAYLOAD_LINES {
        text.push_str(&format!("row{i:05}\t{}\n", "x".repeat(40)));
    }
    text
}

/// The `sed '1d; s/^/>/'` transform the probes apply, computed in Rust
/// so the assertion is byte-exact rather than a shape check.
fn transformed(payload: &str) -> String {
    let mut out = String::new();
    for (index, line) in payload.lines().enumerate() {
        if index == 0 {
            continue;
        }
        out.push('>');
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn run_script(dir: &Path, body: &str, env: &[(&str, &Path)]) -> (String, String, Option<i32>) {
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write probe script");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(&script)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn rubash");
    let mut out_pipe = child.stdout.take().expect("stdout pipe");
    let mut err_pipe = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash hung (>60s) running probe script:\n{body}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let stdout = String::from_utf8_lossy(&out_thread.join().expect("stdout drain")).into_owned();
    let stderr = String::from_utf8_lossy(&err_thread.join().expect("stderr drain")).into_owned();
    (stdout, stderr, status.code())
}

/// The issue's main form: function producer | `command sed` inside a
/// command substitution, payload above the deadlock threshold. GNU
/// completes in ~25 ms with the full transformed text.
#[test]
fn comsub_pipeline_above_pipe_threshold_does_not_deadlock() {
    let dir = scratch("comsub-big");
    let payload = payload();
    let tab = dir.join("tab.txt");
    std::fs::write(&tab, &payload).expect("write payload");
    let (stdout, stderr, code) = run_script(
        &dir,
        "dl() { command cat \"$TAB\"; }\nout=$(dl | command sed '1d; s/^/>/')\nprintf '%s' \"$out\" > result.txt\n",
        &[("TAB", &tab)],
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let result = std::fs::read_to_string(dir.join("result.txt")).expect("result written");
    // The substitution strips trailing newlines; the transform ends in
    // exactly one, so the capture is the transform minus its last byte.
    let expected = transformed(&payload);
    assert_eq!(
        result,
        expected.strip_suffix('\n').expect("trailing newline"),
        "captured output diverges from the GNU transform"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The file-endpoint sibling (audit form v3): the same pipeline NOT in
/// a substitution, final output redirected to a file. Before the fix
/// this returned status 0 with the output entirely lost (0 bytes).
#[test]
fn pipeline_to_file_above_pipe_threshold_keeps_output() {
    let dir = scratch("file-big");
    let payload = payload();
    let tab = dir.join("tab.txt");
    std::fs::write(&tab, &payload).expect("write payload");
    let (stdout, stderr, code) = run_script(
        &dir,
        "dl() { command cat \"$TAB\"; }\ndl | command sed '1d; s/^/>/' > o.txt\n",
        &[("TAB", &tab)],
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let result = std::fs::read_to_string(dir.join("o.txt")).expect("output file written");
    assert_eq!(
        result,
        transformed(&payload),
        "file output diverges from the GNU transform"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Guard (passed before the fix): the small-input comsub form must keep
/// its content byte-exact — the drain change must not perturb the
/// ordinary path.
#[test]
fn comsub_pipeline_small_input_content_intact() {
    let dir = scratch("comsub-small");
    let (stdout, stderr, code) = run_script(
        &dir,
        "out=$(printf 'a\\nb\\nc\\n' | command sed 's/^/>/')\nprintf '[%s]\\n' \"$out\"\n",
        &[],
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "[>a\n>b\n>c]\n", "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Guard (passed before the fix): the small-input file-endpoint form.
#[test]
fn pipeline_to_file_small_input_content_intact() {
    let dir = scratch("file-small");
    std::fs::write(dir.join("small.txt"), "a\nb\nc\n").expect("write small payload");
    let (stdout, stderr, code) = run_script(
        &dir,
        "dl() { command cat small.txt; }\ndl | command sed 's/^/>/' > o.txt\n",
        &[],
    );
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let result = std::fs::read_to_string(dir.join("o.txt")).expect("output file written");
    assert_eq!(result, ">a\n>b\n>c\n");
    let _ = std::fs::remove_dir_all(&dir);
}
