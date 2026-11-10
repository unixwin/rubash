//! rubash#2793: /dev/tcp works in-process, but a socket fd >= 3 was
//! not delivered to external children, and /dev/udp sends silently
//! dropped datagrams (reported on Windows niu 1.5.0; WSL GNU bash
//! 5.3.20 as the oracle: `cat <&3` on a /dev/tcp fd reads the full
//! response, and a /dev/udp datagram arrives).
//!
//! GNU anchors: redir.c:660 redir_open routes /dev/tcp|udp targets
//! through netopen (lib/sh/netopen.c:282), which leaves ONE ordinary
//! descriptor in the fd table — every later dup2/inherit treats it
//! like any other descriptor (execute_cmd.c redirection apply), so
//! an external child's `<&3` dup reads the socket, and a write to a
//! /dev/udp descriptor sends a datagram on the connected socket.
//!
//! Status on master 88047f50 (2026-11-08): the Linux face of every
//! cell below is verified GNU-equal by hand (loopback servers, this
//! file's own shapes) — these pins lock that in. On Windows the two
//! defect cells (external cat over a socket fd, UDP delivery) are
//! the regression pins for the Windows delivery face; the in-process
//! and plain-file cells are the controls from the issue's own
//! isolation table.

use std::io::{Read, Write};
use std::net::{TcpListener, UdpSocket};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Bounded `-c` runner (poll + kill past the deadline, then drain).
fn run_c(script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(25);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within 25s");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.code())
}

/// One-shot HTTP-ish loopback server: accepts one connection, reads
/// the request, replies with a fixed response, closes.
fn start_tcp_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind tcp server");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.0 200 OK\r\n\r\nhello-body");
            let _ = stream.flush();
        }
    });
    port
}

/// Defect cell 1 (the issue's isolation table, third row): an
/// external `cat` dup'ing a /dev/tcp fd must receive the response —
/// the socket descriptor rides the same delivery channel as a file.
#[test]
fn issue2793_external_cat_reads_devtcp_fd() {
    let port = start_tcp_server();
    let (stdout, stderr, code) = run_c(&format!(
        "exec 3<>/dev/tcp/127.0.0.1/{port}\n\
         printf 'GET /hello.txt HTTP/1.0\\r\\n\\r\\n' >&3\n\
         out=$(cat <&3)\n\
         exec 3<&- 3>&-\n\
         echo \"got:[$out]\"\n"
    ));
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert!(
        stdout.starts_with("got:[HTTP/1.0 200 OK"),
        "stdout: {stdout:?}"
    );
    assert!(stdout.contains("hello-body"), "stdout: {stdout:?}");
}

/// Control (isolation table, first row): the builtin read on the
/// same fd — the in-process face that already worked.
#[test]
fn issue2793_builtin_read_devtcp_fd_control() {
    let port = start_tcp_server();
    let (stdout, stderr, code) = run_c(&format!(
        "exec 3<>/dev/tcp/127.0.0.1/{port}\n\
         printf 'GET /hello.txt HTTP/1.0\\r\\n\\r\\n' >&3\n\
         read -r first <&3\n\
         exec 3<&- 3>&-\n\
         echo \"first:[$first]\"\n"
    ));
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "first:[HTTP/1.0 200 OK\r]\n", "stderr:\n{stderr}");
}

/// Control (isolation table, second row): the same external `cat <&3`
/// shape over a plain-file fd — the delivery channel itself.
#[test]
fn issue2793_external_cat_reads_file_fd_control() {
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash2793-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("hello.txt"), "file-body\n").expect("write fixture");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-c", "exec 3< hello.txt; cat <&3; exec 3<&-"])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "file-body\n", "stderr:\n{stderr}");
}

/// Defect cell 2: a write to /dev/udp must put a datagram on the
/// wire (GNU netopen connects the UDP socket; the write sends).
/// Silent success with nothing arriving is the failure being pinned.
#[test]
fn issue2793_udp_redirect_delivers_datagram() {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind udp listener");
    let port = socket.local_addr().expect("local addr").port();
    socket
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set timeout");
    let (stdout, stderr, code) = run_c(&format!(
        "echo ping > /dev/udp/127.0.0.1/{port}\n\
         echo \"rc:$?\"\n"
    ));
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc:0\n", "stderr:\n{stderr}");
    let mut buf = [0u8; 64];
    let (n, _peer) = socket
        .recv_from(&mut buf)
        .expect("no datagram arrived for the /dev/udp write");
    assert_eq!(&buf[..n], b"ping\n");
}
