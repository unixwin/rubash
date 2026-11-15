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
//!
//! W-2793 facecheck extension (2026-11-10, master beab07dc): two
//! more GNU-measured cells — a genuinely foreign child (python3)
//! reading its inherited fd 3 directly (no stdin dup), and the
//! /dev/udp fd form (`exec 3<>/dev/udp/...` + `printf >&3` +
//! `read -u 3` of the reply). GNU bash 5.3.20 oracle, script-file
//! probes: the foreign child prints the full response from fd 3,
//! and the UDP server receives `ping-udp-fd` on the fd form just
//! as on the direct form. The Windows face of the issue remains
//! open (see the lane REPORT): on Windows a numbered fd >= 3 has
//! no live delivery channel to a foreign child in the spawn
//! model, so these cells are the pins that face must satisfy.

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

/// Extension cell (W-2793): a genuinely foreign child — python3,
/// not any shell-side emulation — inherits fd 3 itself and reads
/// the socket stream from that number. GNU's fork hands the child
/// the descriptor at its number (execute_cmd.c fork inheritance);
/// the delivery channel must do the same for a socket fd, not only
/// for the stdin dup of the `cat <&3` cell. Skips when python3 is
/// absent (the cell's foreign reader is then unavailable).
#[test]
fn issue2793_foreign_child_reads_inherited_devtcp_fd3() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let dir: PathBuf = std::env::temp_dir().join(format!("rubash2793-fd3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let helper = dir.join("read_fd3.py");
    std::fs::write(
        &helper,
        "import os, sys\n\
         data = b''\n\
         while True:\n\
         \x20   chunk = os.read(3, 4096)\n\
         \x20   if not chunk: break\n\
         \x20   data += chunk\n\
         sys.stdout.write('FD3-GOT:[' + data.decode(errors='replace') + ']\\n')\n",
    )
    .expect("write helper");
    let port = start_tcp_server();
    let (stdout, stderr, code) = run_c(&format!(
        "exec 3<>/dev/tcp/127.0.0.1/{port}\n\
         printf 'GET /hello.txt HTTP/1.0\\r\\n\\r\\n' >&3\n\
         python3 {}\n\
         exec 3<&- 3>&-\n",
        helper.display()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert!(
        stdout.starts_with("FD3-GOT:[HTTP/1.0 200 OK"),
        "stdout: {stdout:?}"
    );
    assert!(stdout.contains("hello-body"), "stdout: {stdout:?}");
}

/// Extension cell (W-2793): the /dev/udp fd form — `exec 3<>`
/// binds the connected datagram socket, `printf >&3` sends one
/// datagram, and the peer's reply is readable on the same fd
/// (GNU netopen: one descriptor, both directions). The direct
/// `echo > /dev/udp` cell above covers the one-shot form; this
/// covers the persistent-fd form the issue's second defect also
/// implicates.
///
/// The reply readback is locked to GNU's byte-wise read, not to
/// a whole-datagram read: the plain line path of `read` consumes
/// input one byte per read(2) (builtins/read.def:736,
/// `zread (fd, &c, 1)`), so on a datagram socket the first byte's
/// recv consumes the entire packet and the kernel discards the
/// rest; the read then waits for a further datagram until `-t`
/// expires, returning 142 (128 + SIGALRM) with `line` holding
/// just `p`. GNU bash 5.3.20 measured exactly this shape
/// (rc 142, `line:[p]`) at -t 5 and at the -t 1 used here;
/// rubash matches byte-for-byte. The earlier whole-line
/// expectation was never GNU-measured (delimitation record in
/// the lane report). A stream face reads the full line back;
/// that is a different shape, not this cell.
#[test]
fn issue2793_udp_fd_sends_datagram_and_reads_reply() {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind udp listener");
    let port = socket.local_addr().expect("local addr").port();
    socket
        .set_read_timeout(Some(Duration::from_secs(15)))
        .expect("set timeout");
    let server = std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        let (n, peer) = socket.recv_from(&mut buf).expect("no datagram arrived");
        let got = buf[..n].to_vec();
        let _ = socket.send_to(b"pong-udp\n", peer);
        got
    });
    let (stdout, stderr, code) = run_c(&format!(
        "exec 3<>/dev/udp/127.0.0.1/{port}\n\
         printf 'ping-udp-fd\\n' >&3\n\
         read -t 1 -u 3 line\n\
         echo \"rc:$?\"\n\
         echo \"line:[$line]\"\n\
         exec 3<&- 3>&-\n"
    ));
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc:142\nline:[p]\n", "stderr:\n{stderr}");
    let received = server.join().expect("udp server thread");
    assert_eq!(received, b"ping-udp-fd\n".to_vec());
}
