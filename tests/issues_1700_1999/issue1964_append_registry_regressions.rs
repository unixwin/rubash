//! Issue rubash#1964: the append/probe ladders bypassed the
//! special-path registry — `probe_output_redirect_append`, the
//! no-output mirror's append arms, the external-command append arm
//! and the output routing's validation/flush opens all raw-opened
//! the MAPPED literal path. A `/dev/tcp` append target was therefore
//! unreachable through them (the mapped name is not a file: the
//! probe killed the redirect with ENOENT before any real open, and
//! preempted external commands whose spawn would have connected),
//! and a synthetic /proc append landed on the enclave literal path.
//! Every append-shaped open now goes through
//! `Executor::open_append_target`, which classifies first
//! (redir.c:702 redir_open performs one classified open of the
//! word).
//!
//! GNU 5.3.20 oracle shapes: `>> /dev/tcp/host/port` connects and
//! delivers; a refused port reports "Connection refused" naming the
//! target; `>>` on a plain file appends.
//!
//! Note on connection counts: rubash's setup walk + mirror/flush
//! split performs the open more than once for builtin faces (the
//! pre-existing double-open the mirror's own comments document), so
//! a loopback server may see an extra empty connection before the
//! delivering one. The pins assert delivery and acceptance, not
//! connection counts.

use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1964-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn rubash_c_with_stdin(script: &str, dir: &Path, payload: &[u8]) -> Run {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(payload)
        .expect("feed stdin");
    let output = child.wait_with_output().expect("wait rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// A loopback sink: accepts connections until the deadline, collects
/// every byte read from all of them, and counts connections.
struct Sink {
    port: u16,
    handle: std::thread::JoinHandle<(usize, Vec<u8>)>,
}

fn start_sink() -> Sink {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind sink");
    let port = listener.local_addr().expect("sink addr").port();
    listener.set_nonblocking(true).expect("nonblocking");
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut connections = 0usize;
        let mut bytes = Vec::new();
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    connections += 1;
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut buf = Vec::new();
                    let _ = stream.read_to_end(&mut buf);
                    bytes.extend_from_slice(&buf);
                    // Stop early once the shell under test is done:
                    // callers use one sink per rubash run, and a
                    // short quiet period after the last connection
                    // means the run is over.
                    if !buf.is_empty() {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if connections > 0 {
                        // A beat of quiet after at least one
                        // connection: the run has finished.
                        std::thread::sleep(Duration::from_millis(400));
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => break,
            }
        }
        (connections, bytes)
    });
    Sink { port, handle }
}

fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = listener.local_addr().expect("probe addr").port();
    drop(listener);
    port
}

#[test]
fn issue1964_no_output_append_to_tcp_connects() {
    let dir = tempdir("colon-append");
    let sink = start_sink();
    let run = rubash_c(&format!(": >> /dev/tcp/127.0.0.1/{}", sink.port), &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    let (connections, _bytes) = sink.handle.join().expect("sink thread");
    assert!(connections >= 1, "the append probe never connected");
}

#[test]
fn issue1964_external_append_to_tcp_delivers() {
    let dir = tempdir("external-append");
    let sink = start_sink();
    let run = rubash_c_with_stdin(
        &format!("cat >> /dev/tcp/127.0.0.1/{}", sink.port),
        &dir,
        b"append-payload\n",
    );
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    let (_connections, bytes) = sink.handle.join().expect("sink thread");
    assert_eq!(bytes, b"append-payload\n");
}

#[test]
fn issue1964_builtin_echo_append_to_tcp_delivers() {
    let dir = tempdir("echo-append");
    let sink = start_sink();
    let run = rubash_c(
        &format!("echo builtin-payload >> /dev/tcp/127.0.0.1/{}", sink.port),
        &dir,
    );
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    let (_connections, bytes) = sink.handle.join().expect("sink thread");
    assert_eq!(bytes, b"builtin-payload\n");
}

#[test]
fn issue1964_append_to_refused_tcp_reports_refusal() {
    let dir = tempdir("refused");
    let port = closed_port();
    let run = rubash_c(&format!(": >> /dev/tcp/127.0.0.1/{port}"), &dir);
    assert_eq!(run.code, Some(1), "stdout={}", run.stdout);
    assert!(
        run.stderr.contains("Connection refused"),
        "stderr={}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("No such file or directory"),
        "the probe's old literal-path ENOENT leaked: stderr={}",
        run.stderr
    );
}

#[test]
fn issue1964_readwrite_probe_to_tcp_connects() {
    let dir = tempdir("rw-probe");
    let sink = start_sink();
    let run = rubash_c(&format!(": <> /dev/tcp/127.0.0.1/{}", sink.port), &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    let (connections, _bytes) = sink.handle.join().expect("sink thread");
    assert!(connections >= 1, "the <> probe never connected");
}

#[test]
fn issue1964_plain_file_append_unchanged() {
    let dir = tempdir("plain");
    let run = rubash_c("echo one >> f.txt; echo two >> f.txt; cat f.txt", &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert_eq!(run.stdout, "one\ntwo\n");
}

#[cfg(windows)]
#[test]
fn issue1964_synthetic_append_answers_by_write_class() {
    let dir = tempdir("synthetic");
    let run = rubash_c(": >> /proc/version; echo should-not-print", &dir);
    assert_eq!(run.code, Some(1), "stdout={}", run.stdout);
    assert!(
        run.stderr.contains("/proc/version: Permission denied"),
        "stderr={}",
        run.stderr
    );
    let run = rubash_c(": >> /proc/self/status; echo appended-ok", &dir);
    assert_eq!(run.code, Some(0), "stderr={}", run.stderr);
    assert!(run.stdout.contains("appended-ok"), "stdout={}", run.stdout);
}
