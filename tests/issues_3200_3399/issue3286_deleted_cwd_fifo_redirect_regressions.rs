//! rubash#3286: output/append redirection to an existing FIFO from a
//! deleted cwd.
//!
//! GNU (bash 5.3.20) opens the redirect target once, in the child
//! (redir.c:237 do_redirections), resolving the relative operand from
//! the cwd inode — so from a deleted cwd, `echo hello > ../fifo`
//! blocks for the reader's rendezvous and then succeeds, and with no
//! reader it blocks (standard FIFO write-open semantics). rubash's
//! anchor carve-out (#3268) classified an existing FIFO out of the
//! anchor open and fell back to the joined string path, which dies at
//! the deleted component: both shapes failed instantly with ENOENT.
//!
//! The fix splits the two phases: the BIND opens
//! (create_redirect_output / open_append_target) take the anchor
//! O_WRONLY openat for a FIFO, while the PROBE faces (setup-walk
//! probes, the compound prepared phase, the #1868 validation walk)
//! validate a FIFO by anchor faccessat and never open it (#1820).
//!
//! These pins assert COMPLETION FORMS (bytes + exit status) and, for
//! the no-reader shape, the BLOCKING FORM (the run is still alive at
//! the watchdog deadline instead of failing fast). Unix only: the cwd
//! anchor is a Unix directory handle. Every run carries a watchdog
//! (own process group, single-process kill) so a hung shape fails the pin
//! instead of stalling the suite (rubash#1868).

#![cfg(unix)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

fn mkfifo(path: &std::path::Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("spawn mkfifo");
    assert!(status.success(), "mkfifo {path:?}");
}

/// Scratch root D: D/a is the directory the shell enters and loses,
/// D/fifo is the prebuilt FIFO in the parent.
fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-3286-{case}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).expect("create a");
    mkfifo(&dir.join("fifo"));
    ScratchDir(dir)
}

/// A reader that holds the FIFO's read end: its open blocks until a
/// writer opens (the rendezvous), then it reads to EOF and reports
/// the bytes. The thread stays blocked on the open when no writer
/// ever arrives; the receiver's timeout bounds the test instead.
fn spawn_fifo_reader(path: std::path::PathBuf) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok(mut file) = std::fs::File::open(&path) else {
            return;
        };
        let mut bytes = Vec::new();
        let _ = file.read_to_end(&mut bytes);
        let _ = sender.send(bytes);
    });
    receiver
}

/// Drain one pipe on a thread, forwarding chunks as they arrive so
/// the harness never depends on every write end closing (rubash#1868).
fn drain_pipe<R: Read + Send + 'static>(mut pipe: R) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if sender.send(chunk[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    receiver
}

fn collect_drained(receiver: mpsc::Receiver<Vec<u8>>, grace: Duration) -> Vec<u8> {
    let mut bytes = Vec::new();
    let deadline = Instant::now() + grace;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match receiver.recv_timeout(remaining) {
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    bytes
}

/// Run `script` (written to D/case.sh) under rubash with cwd D and a
/// watchdog deadline. Returns (stdout, stderr, status); status is
/// None when the run was still alive at the deadline and the whole
/// process group was killed.
fn run_script_watchdog(
    dir: &std::path::Path,
    script: &str,
    timeout: Duration,
) -> (String, String, Option<std::process::ExitStatus>) {
    run_script_watchdog_grace(dir, script, timeout, None)
}

/// `run_script_watchdog` with an explicit post-exit drain grace: the
/// background-job pin leaves an orphaned child holding the script's
/// pipes, so its drains need a short grace instead of the default.
fn run_script_watchdog_grace(
    dir: &std::path::Path,
    script: &str,
    timeout: Duration,
    drain_grace: Option<Duration>,
) -> (String, String, Option<std::process::ExitStatus>) {
    use std::os::unix::process::CommandExt;
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let mut child = Command::new(rubash_bin())
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", dir)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .expect("spawn rubash");
    let stdout_rx = drain_pipe(child.stdout.take().expect("stdout pipe"));
    let stderr_rx = drain_pipe(child.stderr.take().expect("stderr pipe"));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status),
            None => {
                if Instant::now() >= deadline {
                    // Single-process kill only: a process-GROUP kill
                    // (`/bin/kill -KILL -pgid`, the #1820 pattern)
                    // takes the caller's session down with it on this
                    // box, so it must never fire here. The watchdog
                    // runs hold only the shell itself in the group
                    // (the blocked redirect is a builtin), so killing
                    // the child suffices; the pipes then hit EOF and
                    // the drains end.
                    let _ = child.kill();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let _ = child.wait();
    let grace = drain_grace.unwrap_or_else(|| {
        if status.is_some() {
            Duration::from_secs(10)
        } else {
            Duration::from_secs(5)
        }
    });
    let stdout = collect_drained(stdout_rx, grace);
    let stderr = collect_drained(stderr_rx, grace);
    (
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
        status,
    )
}

/// The deleted-cwd staging shared by every pin: enter D/a, then a
/// subshell steps out and removes it, leaving the shell on the dead
/// inode (rubash#3268 staging).
const DELETED_CWD_PRELUDE: &str = "cd a\n(cd ..; rmdir a)\n";

#[test]
fn issue3286_deleted_cwd_fifo_output_reader_present() {
    let dir = scratch("output-reader");
    let reader = spawn_fifo_reader(dir.0.join("fifo"));
    let script = format!("{DELETED_CWD_PRELUDE}echo hello > ../fifo\necho \"rc=$?\"\n");
    let (stdout, stderr, status) = run_script_watchdog(&dir.0, &script, Duration::from_secs(15));
    assert!(
        status.is_some_and(|s| s.success()),
        "run must complete: {stdout} {stderr}"
    );
    assert_eq!(stdout, "rc=0\n", "redirect must succeed");
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
    let got = reader
        .recv_timeout(Duration::from_secs(10))
        .expect("reader must receive the bytes");
    assert_eq!(got, b"hello\n");
}

#[test]
fn issue3286_deleted_cwd_fifo_output_no_reader_blocks() {
    let dir = scratch("output-noreader");
    let script = format!("{DELETED_CWD_PRELUDE}echo hello > ../fifo\necho \"rc=$?\"\n");
    let started = Instant::now();
    let (stdout, stderr, status) = run_script_watchdog(&dir.0, &script, Duration::from_secs(4));
    assert!(
        status.is_none(),
        "with no reader the write open must block, not fail fast: stdout={stdout} stderr={stderr}"
    );
    assert!(
        started.elapsed() >= Duration::from_secs(3),
        "the run must have been alive for the whole watchdog window"
    );
    assert!(
        !stdout.contains("rc=1"),
        "the command must never reach its next line: {stdout}"
    );
    assert!(
        !stderr.contains("No such file or directory"),
        "no instant ENOENT: {stderr}"
    );
}

#[test]
fn issue3286_deleted_cwd_fifo_append_reader_present() {
    let dir = scratch("append-reader");
    let reader = spawn_fifo_reader(dir.0.join("fifo"));
    let script = format!("{DELETED_CWD_PRELUDE}echo hello >> ../fifo\necho \"rc=$?\"\n");
    let (stdout, stderr, status) = run_script_watchdog(&dir.0, &script, Duration::from_secs(15));
    assert!(
        status.is_some_and(|s| s.success()),
        "run must complete: {stdout} {stderr}"
    );
    assert_eq!(stdout, "rc=0\n", "append must succeed");
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
    let got = reader
        .recv_timeout(Duration::from_secs(10))
        .expect("reader must receive the bytes");
    assert_eq!(got, b"hello\n");
}

#[test]
fn issue3286_deleted_cwd_fifo_append_no_reader_blocks() {
    let dir = scratch("append-noreader");
    let script = format!("{DELETED_CWD_PRELUDE}echo hello >> ../fifo\necho \"rc=$?\"\n");
    let (stdout, stderr, status) = run_script_watchdog(&dir.0, &script, Duration::from_secs(4));
    assert!(
        status.is_none(),
        "with no reader the append open must block, not fail fast: stdout={stdout} stderr={stderr}"
    );
    assert!(
        !stderr.contains("No such file or directory"),
        "no instant ENOENT: {stderr}"
    );
}

#[test]
fn issue3286_deleted_cwd_fifo_output_compound_inner() {
    // The redirect sits on the inner echo, not on the group: the
    // simple-command faces (probe + bind + flush) own it.
    let dir = scratch("compound-inner");
    let reader = spawn_fifo_reader(dir.0.join("fifo"));
    let script = format!("{DELETED_CWD_PRELUDE}{{ echo hello > ../fifo; }}\necho \"rc=$?\"\n");
    let (stdout, stderr, status) = run_script_watchdog(&dir.0, &script, Duration::from_secs(15));
    assert!(
        status.is_some_and(|s| s.success()),
        "run must complete: {stdout} {stderr}"
    );
    assert_eq!(stdout, "rc=0\n", "inner redirect must succeed");
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
    let got = reader
        .recv_timeout(Duration::from_secs(10))
        .expect("reader must receive the bytes");
    assert_eq!(got, b"hello\n");
}

#[test]
fn issue3286_deleted_cwd_fifo_output_empty_stream_pairs() {
    // GNU opens the redirect even when nothing is written (`echo -n
    // "" > fifo` blocks for a reader and hands it EOF); the pairing
    // open is the flush site's, not the probe's. (`printf ""` renders
    // zero output chunks and never reaches the flush at all — a
    // separate, pre-existing live-cwd gap, not this issue's cell.)
    let dir = scratch("empty-stream");
    let reader = spawn_fifo_reader(dir.0.join("fifo"));
    let script = format!("{DELETED_CWD_PRELUDE}echo -n \"\" > ../fifo\necho \"rc=$?\"\n");
    let (stdout, stderr, status) = run_script_watchdog(&dir.0, &script, Duration::from_secs(15));
    assert!(
        status.is_some_and(|s| s.success()),
        "run must complete: {stdout} {stderr}"
    );
    assert_eq!(stdout, "rc=0\n", "empty write must succeed");
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
    let got = reader
        .recv_timeout(Duration::from_secs(10))
        .expect("reader must observe the EOF pairing");
    assert!(got.is_empty(), "no bytes, just the EOF pairing");
}

#[test]
fn issue3286_deleted_cwd_regular_file_output_unaffected() {
    let dir = scratch("regular-file");
    let script = format!("{DELETED_CWD_PRELUDE}echo hello > ../g\necho \"rc=$?\"\n");
    let (stdout, stderr, status) = run_script_watchdog(&dir.0, &script, Duration::from_secs(15));
    assert!(
        status.is_some_and(|s| s.success()),
        "run must complete: {stdout} {stderr}"
    );
    assert_eq!(stdout, "rc=0\n");
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.0.join("g")).expect("read g"),
        "hello\n"
    );
}

#[test]
fn issue3286_live_cwd_fifo_output_unaffected() {
    let dir = scratch("live-cwd");
    let reader = spawn_fifo_reader(dir.0.join("fifo"));
    let script = "echo hello > fifo\necho \"rc=$?\"\n";
    let (stdout, stderr, status) = run_script_watchdog(&dir.0, script, Duration::from_secs(15));
    assert!(
        status.is_some_and(|s| s.success()),
        "run must complete: {stdout} {stderr}"
    );
    assert_eq!(stdout, "rc=0\n");
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
    let got = reader
        .recv_timeout(Duration::from_secs(10))
        .expect("reader must receive the bytes");
    assert_eq!(got, b"hello\n");
}

#[test]
fn issue3286_deleted_cwd_fifo_output_external_command() {
    let dir = scratch("external");
    let reader = spawn_fifo_reader(dir.0.join("fifo"));
    let script = format!("{DELETED_CWD_PRELUDE}/bin/echo hello > ../fifo\necho \"rc=$?\"\n");
    let (stdout, stderr, status) = run_script_watchdog(&dir.0, &script, Duration::from_secs(15));
    assert!(
        status.is_some_and(|s| s.success()),
        "run must complete: {stdout} {stderr}"
    );
    assert_eq!(stdout, "rc=0\n", "external bind must succeed");
    assert!(stderr.is_empty(), "no diagnostics expected: {stderr}");
    let got = reader
        .recv_timeout(Duration::from_secs(10))
        .expect("reader must receive the bytes");
    assert_eq!(got, b"hello\n");
}

#[test]
fn issue3286_deleted_cwd_missing_target_still_fails() {
    let dir = scratch("missing-target");
    let script = format!("{DELETED_CWD_PRELUDE}echo hello > ../nope/x\necho \"rc=$?\"\n");
    let (stdout, stderr, status) = run_script_watchdog(&dir.0, &script, Duration::from_secs(15));
    assert!(
        status.is_some(),
        "a missing parent must fail fast, not block: {stdout} {stderr}"
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("No such file or directory"),
        "GNU ENOENT diagnostic expected: {stderr}"
    );
}

#[test]
fn issue3286_deleted_cwd_fifo_output_background_job_does_not_block_parent() {
    // #1862 discipline under a deleted cwd: the parent must not take
    // the FIFO rendezvous for a background job — the open belongs to
    // the child, so the parent sails on with no reader present.
    let dir = scratch("background");
    let script = format!(
        "{DELETED_CWD_PRELUDE}echo bg > ../fifo &\necho \"bg_rc=$?\"\nsleep 1\necho survived\n"
    );
    let (stdout, stderr, status) = run_script_watchdog_grace(
        &dir.0,
        &script,
        Duration::from_secs(8),
        Some(Duration::from_secs(1)),
    );
    assert!(
        status.is_some_and(|s| s.success()),
        "the parent must not block on the background FIFO open: {stdout} {stderr}"
    );
    assert!(stdout.contains("bg_rc=0"), "spawn must succeed: {stdout}");
    assert!(
        stdout.contains("survived"),
        "the parent must reach its next lines: {stdout}"
    );
}
