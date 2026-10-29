//! rubash#2091 — the #1964 residual /dev/tcp faces, re-measured on
//! master after PR #2015 and the #1967/#1968 settlements:
//!
//! - append (`>>`) faces: ALREADY SETTLED on current master (exactly
//!   one connection, delivery intact) — pinned here as controls so the
//!   settlement cannot silently regress.
//! - `read x <> /dev/tcp/...` (and the plain `<` twin): the read
//!   builtin's own serve ladder never classified the net target — it
//!   translated the spelling to a filesystem path and served a silent
//!   EOF (x empty, read status 1, ZERO connections, no diagnostic).
//!   GNU redir.c netopen connects once and read serves the greeting;
//!   a refused connect is a redirection failure with the two-line
//!   diagnostic (`connect: Connection refused` from netopen, then
//!   `<target>: Connection refused` from redirection_error).
//! - external `cat <<EOF <> /dev/tcp/...` and `cat <> /dev/tcp/...`:
//!   the external fd-0 ReadWrite arm opened the translated filesystem
//!   path raw, failing with a bare `No such file or directory` that
//!   named no target, before any connection. The arm now routes net
//!   targets through open_net_redirect like every other class face.
//!
//! Expectations measured against GNU bash 5.3.20 (script files and
//! -c, loopback listener counting connections, 2026-10-29).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

fn run_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2091-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg(&case)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Loopback listener: greets each accepted connection, then records
/// the bytes it receives until the peer closes. Reports
/// (connections, received bytes) continuously like the #1968 pins.
struct Listener {
    port: u16,
    rx: mpsc::Receiver<(usize, Vec<u8>)>,
}

fn start_listener(greeting: Option<&'static str>) -> Listener {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking");
        let mut count = 0usize;
        let mut received: Vec<u8> = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    count += 1;
                    if let Some(greeting) = greeting {
                        let _ = stream.write_all(greeting.as_bytes());
                        let _ = stream.flush();
                    }
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
                    let mut sink = [0u8; 256];
                    loop {
                        match stream.read(&mut sink) {
                            Ok(0) => break,
                            Ok(n) => received.extend_from_slice(&sink[..n]),
                            Err(_) => break,
                        }
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
            if tx.send((count, received.clone())).is_err() {
                break;
            }
        }
    });
    Listener { port, rx }
}

impl Listener {
    fn stats(&self) -> (usize, Vec<u8>) {
        std::thread::sleep(Duration::from_millis(500));
        let mut last = (0, Vec::new());
        while let Ok(stats) = self.rx.try_recv() {
            last = stats;
        }
        last
    }
}

fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

#[test]
fn issue2091_read_readwrite_tcp_reads_greeting_once() {
    let listener = start_listener(Some("GREETING-LINE\n"));
    let (stdout, stderr, code) = run_c(&format!(
        "read x <> /dev/tcp/127.0.0.1/{}; echo \"rc=$? x=[$x]\"",
        listener.port
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 x=[GREETING-LINE]\n", "stdout: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(listener.stats().0, 1, "exactly one connection");
}

#[test]
fn issue2091_read_input_tcp_reads_greeting_once() {
    // The plain `<` twin shares the serve ladder and was broken the
    // same way (silent EOF, zero connections).
    let listener = start_listener(Some("GREETING-LINE\n"));
    let (stdout, stderr, code) = run_c(&format!(
        "read x < /dev/tcp/127.0.0.1/{}; echo \"rc=$? x=[$x]\"",
        listener.port
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 x=[GREETING-LINE]\n", "stdout: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(listener.stats().0, 1, "exactly one connection");
}

#[test]
fn issue2091_read_tcp_refused_is_a_redirection_failure() {
    let port = closed_port();
    let (stdout, stderr, code) = run_c(&format!(
        "read x <> /dev/tcp/127.0.0.1/{port}; echo \"rc=$? x=[$x]\""
    ));
    assert_eq!(code, Some(0), "the script itself completes");
    assert_eq!(stdout, "rc=1 x=[]\n", "stdout: {stdout}");
    assert!(
        stderr.contains("connect: Connection refused"),
        "netopen's own line: stderr: {stderr}"
    );
    assert!(
        stderr.contains(&format!("/dev/tcp/127.0.0.1/{port}: Connection refused")),
        "redirection_error names the target: stderr: {stderr}"
    );
}

#[test]
fn issue2091_external_heredoc_readwrite_tcp_reads_greeting() {
    let listener = start_listener(Some("GREETING-LINE\n"));
    let (stdout, stderr, code) = run_script(
        "hd",
        &format!(
            "cat <<EOF <> /dev/tcp/127.0.0.1/{}\nignored\nEOF\n",
            listener.port
        ),
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "GREETING-LINE\n", "stdout: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(listener.stats().0, 1, "exactly one connection");
}

#[test]
fn issue2091_external_readwrite_tcp_reads_greeting() {
    let listener = start_listener(Some("GREETING-LINE\n"));
    let (stdout, stderr, code) = run_c(&format!("cat <> /dev/tcp/127.0.0.1/{}", listener.port));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "GREETING-LINE\n", "stdout: {stdout}");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(listener.stats().0, 1, "exactly one connection");
}

#[test]
fn issue2091_external_readwrite_tcp_refused_names_target() {
    let port = closed_port();
    let (stdout, stderr, code) = run_c(&format!("cat <> /dev/tcp/127.0.0.1/{port}; echo rc=$?"));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains(&format!("/dev/tcp/127.0.0.1/{port}: Connection refused")),
        "the diagnostic must name the target: stderr: {stderr}"
    );
}

#[test]
fn issue2091_append_tcp_stays_one_connection_with_delivery() {
    // Control cells: the append faces were already settled on master
    // before this batch (one connection, bytes delivered). Pin the
    // settlement so it cannot regress behind the read/external fixes.
    let listener = start_listener(None);
    let (stdout, _stderr, code) = run_c(&format!(
        ": >> /dev/tcp/127.0.0.1/{0}; echo hi >> /dev/tcp/127.0.0.1/{0}; /bin/echo hi2 >> /dev/tcp/127.0.0.1/{0}",
        listener.port
    ));
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "", "stdout: {stdout}");
    let (connections, received) = listener.stats();
    assert_eq!(connections, 3, "one connection per append command");
    assert_eq!(
        String::from_utf8_lossy(&received),
        "hi\nhi2\n",
        "both payloads delivered"
    );
}
