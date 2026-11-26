//! rubash#586: the `-s` stdin-script entry must read commands ON DEMAND, one
//! complete command at a time, never pre-buffering the stream.
//!
//! GNU ground truth (vendored bash 5.3): the parser's buffered stream over a
//! non-seekable stdin has capacity ONE (input.c:381-397 fd_to_buffered_stream
//! `size = fd_is_seekable(fd) ? min(st_size, MAX_INPUT_BUFFER_SIZE) : 1`,
//! input.c:192-193 B_UNBUFF), and a seekable script stream is rewound to the
//! first unconsumed byte before any fork (input.c:552-565
//! sync_buffered_stream, from jobs.c:2298-2302 make_child and input.c:296-316
//! check_bash_input). So a consumer that reads fd 0 — a `$(exec cat)` / `$(cat)`
//! capture, a bare `read`, a top-level `exec cat` — sees exactly the bytes
//! after the last parser-submitted command, consumes them as DATA, and those
//! bytes never fall back to code. The #583 exec fix made the child-stdin
//! channel correct for the `-c` entry; the stdin-script entry needs the whole
//! reader to stay incremental — a host route that slurps the stream up front
//! (`read_to_string` / a BufReader) hands `$(exec cat)` an empty capture and
//! re-arms every later line as code (data-as-code: the `curl | bash -s`
//! install-script hazard this issue pins).
//!
//! Every expected output below was byte-verified against WSL GNU bash
//! 5.3.0(1)-release with the identical stdin.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_stdin_file(script: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join("rubash586_stdin_script.sh");
    std::fs::write(&path, script).expect("write temp script");
    let file = std::fs::File::open(&path).expect("open temp script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .stdin(Stdio::from(file))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's own repro: `$(exec cat)` captures everything AFTER its own
/// command line (the `echo "v=[$v]"` and both MARKER lines) as data, so the
/// capture is non-empty, the echo line never runs as code, and no MARKER
/// diagnostic ever fires. GNU: stdout `one\n`, rc 0. A pre-buffering -s entry
/// reads `v=[]` and executes both MARKER lines (rc 127).
#[test]
fn comsub_exec_cat_eats_remainder_as_data() {
    let (stdout, stderr, rc) = run_stdin(
        &["-s"],
        b"echo one\nv=$(exec cat)\necho \"v=[$v]\"\nMARKER-DATA-1\nMARKER-DATA-2\n",
    );
    assert_eq!(stdout, "one\n", "stderr: {stderr:?}");
    assert!(!stderr.contains("MARKER-DATA"), "data lines ran as code");
    assert_eq!(rc, Some(0));
}

/// Top-level `exec cat` replaces the image and cats the remaining script
/// lines as data: `echo never` is PRINTED by cat and never executed. GNU:
/// stdout `one\necho never\n`, rc 0.
#[test]
fn top_level_exec_cat_consumes_remaining_lines() {
    let (stdout, stderr, rc) = run_stdin(&["-s"], b"echo one\nexec cat\necho never\n");
    assert_eq!(stdout, "one\necho never\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// A plain external child between parser reads shares the same cursor: `cat`
/// prints the not-yet-parsed lines as data and they never execute. GNU:
/// stdout `one\necho never\nMARK\n`, rc 0.
#[test]
fn plain_child_cat_consumes_remaining_lines() {
    let (stdout, stderr, rc) = run_stdin(&["-s"], b"echo one\ncat\necho never\nMARK\n");
    assert_eq!(stdout, "one\necho never\nMARK\n", "stderr: {stderr:?}");
    assert!(!stderr.contains("MARK"), "data line ran as code");
    assert_eq!(rc, Some(0));
}

/// `$(cat)` (no exec) sees the same on-demand remainder: the capture swallows
/// the echo line that follows it, so nothing after `one` is printed. GNU:
/// stdout `one\n`, rc 0.
#[test]
fn comsub_plain_cat_reads_remainder() {
    let (stdout, stderr, rc) = run_stdin(&["-s"], b"echo one\nv=$(cat)\necho \"v=[$v]\"\nMARK\n");
    assert_eq!(stdout, "one\n", "stderr: {stderr:?}");
    assert!(!stderr.contains("MARK"), "data line ran as code");
    assert_eq!(rc, Some(0));
}

/// A bare `read` consumes exactly the NEXT line as data, and code RESUMES
/// after the consumed point: `echo after` is eaten by read (never printed),
/// while `TAIL` — never consumed — still executes and fails the shell. GNU:
/// stdout `start\n`, rc 127.
#[test]
fn bare_read_eats_next_line_then_code_resumes() {
    let (stdout, stderr, rc) = run_stdin(&["-s"], b"echo start\nread -r l\necho after\nTAIL\n");
    assert_eq!(stdout, "start\n", "stderr: {stderr:?}");
    assert!(!stdout.contains("after"), "consumed line fell back to code");
    assert_eq!(rc, Some(127), "unconsumed TAIL must execute as code");
}

/// The modernish stdin-script shape that motivated #583 on the `-c` entry:
/// `read` takes the hashbang-adjacent line, then the capture in
/// `eval "$(exec cat)"` receives the remaining lines as its command text.
/// GNU: empty stdout, rc 127 (LINE-A/LINE-B run as the eval'd commands and
/// fail with command-not-found diagnostics naming them).
#[test]
fn read_then_comsub_exec_cat_modernish_shape() {
    let (stdout, stderr, rc) = run_stdin(
        &["-s"],
        b"read -r x\neval \"$(exec cat)\"\nLINE-A\nLINE-B\n",
    );
    assert_eq!(stdout, "", "stderr: {stderr:?}");
    assert!(
        stderr.contains("LINE-A") && stderr.contains("LINE-B"),
        "eval must receive the remainder: {stderr:?}"
    );
    assert_eq!(rc, Some(127));
}

/// Seekable (regular-file) stdin: GNU buffers up to 8192 bytes but rewinds
/// the descriptor before every fork (sync_buffered_stream), so the same
/// on-demand contract holds on the file-redirected `-s` shape. GNU: stdout
/// `one\n`, rc 0.
#[test]
fn file_stdin_exec_cat_eats_remainder_as_data() {
    let (stdout, stderr, rc) =
        run_stdin_file("echo one\nv=$(exec cat)\necho \"v=[$v]\"\nMARKER-DATA-1\nMARKER-DATA-2\n");
    assert_eq!(stdout, "one\n", "stderr: {stderr:?}");
    assert!(!stderr.contains("MARKER-DATA"), "data lines ran as code");
    assert_eq!(rc, Some(0));
}
