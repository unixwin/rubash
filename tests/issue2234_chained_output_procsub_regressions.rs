//! Issue unixwin/rubash#2234: chained output procsub `> >(A) > >(B)`
//!
//! GNU forks the later consumer mid-do_redirections (redir.c:237,
//! subst.c:6362), so the later consumer inherits the earlier
//! substitution's pipe write end as its fd 1/2: its output streams
//! live into the earlier consumer's stdin. Rubash's carrier model
//! delivered a `CarrierWrite` disposition as an append-open of the
//! carrier FILE, but the earlier consumer's input was already
//! snapshotted from that file at finish time — the later consumer's
//! bytes landed after the earlier consumer had read EOF, so the
//! chain printed `0` where GNU prints `2`.
//!
//! The fix (external_setup.rs, `ProcsubLiveStdio`) wires a live pipe
//! for pairs in the same finish batch: the earlier consumer's fd 0
//! becomes the pipe read end (its snapshot bytes are fed in first)
//! and the later consumer's fd 1/2 becomes the pipe write end.
//!
//! Verified against GNU Bash 5.2.21 (box oracle; the issue cites
//! 5.3.20 — the probed shapes agree byte for byte):
//!   `printf 'hi' > >(wc -c) > >(wc -c)`           -> GNU `2`, was `0`
//!   `printf 'hi' > >(wc -c > fa) > >(wc -c)`      -> GNU fa `2`
//!   `printf 'hi' 2> >(wc -c > fa) > >(wc -c >&2)` -> GNU fa `2`
//!   `printf 'hi' > >(wc -c > fa) > >(wc -c) > >(wc -c)` -> GNU fa `2`
//!   `printf 'hi' > >(wc -c > fa) > >(wc -c > fb)` -> GNU fa `0`, fb `2`
//!   `printf 'hi' > >(wc -c > fa) > >(cat | wc -c)` -> GNU fa `2`
//!
//! Out of scope (separate root cause, fd>=3 delivery): a consumer
//! writing to a numbered fd (`>&3`) still fails —
//! `{ echo X >&3; printf hi; } 3> >(wc -c) > >(wc -c >&3)` reports
//! `3: Bad file descriptor` and a lone `3> >(...)` delivers nothing
//! (see the #2234 report: R1/R3/R4 probes). No pin here covers it.
//!
//! Harness note: these pins run the shell as a subprocess and assert
//! on FILES the detached consumers write, polled to a deadline — the
//! consumers are async children whose writes race the shell's exit by
//! design (same harness as tests/issue2195_*). No pin depends on fd 9
//! or on the test process's own fd table shape.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2234-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

/// Poll a file until its content equals `expected` or the deadline
/// passes; returns the last observed content.
fn wait_file_content(path: &Path, expected: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last = String::new();
    while Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(path) {
            last = content;
            if last == expected {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    last
}

// ---------------- #2234 ----------------

#[test]
fn issue2234_chained_stdout_consumers_match_gnu() {
    // The issue's probe, verbatim: the later `wc -c` prints `2` into
    // the earlier consumer's stdin, and the earlier consumer counts
    // those 2 bytes — GNU prints exactly `2`. Pre-fix rubash printed
    // `0` (the later consumer's bytes were appended to the earlier
    // consumer's carrier file after it had read EOF).
    let script = "printf 'hi' > >(wc -c) > >(wc -c)\nwait\n";
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "2\n", "stderr: {stderr}");
}

#[test]
fn issue2234_chained_reader_counts_writer_output_via_file() {
    // Same chain, the reader's count lands in a file so the pin does
    // not depend on stdout plumbing: the writer's `2\n` flows live
    // into the reader, which counts 2 bytes.
    let dir = test_dir("file");
    let fa = dir.join("fa");
    let script = format!(
        "printf 'hi' > >(wc -c > {}) > >(wc -c)\nsleep 0.5\n",
        fa.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
}

#[test]
fn issue2234_stderr_chain_reader_counts_writer_output() {
    // The chain through fd 2: the later consumer's body redirects its
    // stdout onto its fd 2, which the prefix bound to the earlier
    // substitution's pipe — GNU counts `2`, pre-fix rubash `0`.
    let dir = test_dir("stderr");
    let fa = dir.join("fa");
    let script = format!(
        "printf 'hi' 2> >(wc -c > {}) > >(wc -c >&2)\nsleep 0.5\n",
        fa.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
}

#[test]
fn issue2234_triple_chain_counts_through_two_links() {
    // Three in a row: C's `2\n` feeds B, B's `2\n` feeds A, A counts
    // 2 bytes. Each link is its own live pipe.
    let dir = test_dir("triple");
    let fa = dir.join("fa");
    let script = format!(
        "printf 'hi' > >(wc -c > {}) > >(wc -c) > >(wc -c)\nsleep 0.5\n",
        fa.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
}

#[test]
fn issue2234_file_sink_second_consumer_closes_chain() {
    // No chain to feed: the later consumer's body redirects its own
    // fd 1 to a file, closing the pipe write end at startup, so the
    // earlier consumer sees EOF with an empty carrier (`0`) while the
    // later one counts the payload (`2`). Guards the P1 shape.
    let dir = test_dir("filesink");
    let fa = dir.join("fa");
    let fb = dir.join("fb");
    let script = format!(
        "printf 'hi' > >(wc -c > {}) > >(wc -c > {})\nsleep 0.5\n",
        fa.display(),
        fb.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content_a = wait_file_content(&fa, "0\n");
    assert_eq!(content_a, "0\n");
    let content_b = wait_file_content(&fb, "2\n");
    assert_eq!(content_b, "2\n");
}

#[test]
fn issue2234_pipeline_body_writer_feeds_reader() {
    // The writer is a pipeline (`cat | wc -c`): the pipeline's last
    // member writes `2\n` to the reader's stdin through the live
    // pipe; the reader counts 2 bytes.
    let dir = test_dir("pipeline");
    let fa = dir.join("fa");
    let script = format!(
        "printf 'hi' > >(wc -c > {}) > >(cat | wc -c)\nsleep 0.5\n",
        fa.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
}

#[test]
fn issue2234_tee_fanout_unchanged() {
    // Fan-out guard (P5): `tee` inside one consumer still splits the
    // payload to its own inner substitution and its file.
    let dir = test_dir("tee");
    let fo1 = dir.join("fo1");
    let fo2 = dir.join("fo2");
    let script = format!(
        "printf 'hi' > >(tee >(wc -c > {}) > {})\nsleep 0.5\n",
        fo1.display(),
        fo2.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content1 = wait_file_content(&fo1, "2\n");
    assert_eq!(content1, "2\n");
    let content2 = wait_file_content(&fo2, "hi");
    assert_eq!(content2, "hi");
}

#[test]
fn issue2234_nested_chain_unchanged() {
    // Nested guard (P2): the outer consumer's body redirects its own
    // output into an inner substitution — already green, pinned so
    // the live-link change cannot regress it.
    let dir = test_dir("nested");
    let fo = dir.join("fo");
    let script = format!(
        "printf 'hi' > >(cat > >(wc -c > {}))\nsleep 0.5\n",
        fo.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fo, "2\n");
    assert_eq!(content, "2\n");
}
