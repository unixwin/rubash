//! rubash#2102 — writing to a `/dev/tcp` peer that has closed
//! reported `write error: Invalid argument` (EINVAL) where GNU
//! reports the broken-pipe family. Root cause (Windows face, found
//! by the pkg10 F cell against GNU): the builtin flush paths wrote a
//! classified redirect target through std's `File::write_all`, which
//! reaches WriteFile — and WriteFile rejects a SOCKET handle with
//! ERROR_INVALID_PARAMETER outright, live peer or dead, so the real
//! failure never surfaced. The classified opens (`create_redirect_
//! output` / `open_append_target`) adopt the netopen SOCKET as the
//! target File (redir.c netopen, rubash#1026); every flush of such a
//! File now goes through `crate::fd::write_file_all`, the raw-handle
//! primitive whose Windows arm routes sockets through send
//! (rubash#1976) — a dead peer then reports its genuine error
//! (WSAECONNRESET / the EPIPE family) through sh_chkwrite, and a live
//! peer receives the bytes at all.
//!
//! SIGPIPE face (adjudicated against GNU bash 5.3.20 on Linux,
//! 2026-10-29): after a FIN close, the first write succeeds, the
//! second write's EPIPE raises SIGPIPE and the non-interactive shell
//! dies by signal 13 — rubash matches exactly (both measured). After
//! an RST close the first write fails `Connection reset by peer`
//! (ECONNRESET) with status 1 in both shells. Windows has no SIGPIPE:
//! the second write surfaces as the write-error diagnostic instead.
//!
//! These pins are hermetic loopback listeners; on Linux the std and
//! primitive writes are the same write(2), so the Linux value of the
//! pins is plumbing + the SIGPIPE/error-text contract — the EINVAL
//! itself only ever existed on the Windows WriteFile path.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

fn run_script(tag: &str, script: &str) -> (String, String, std::process::ExitStatus) {
    let dir = std::env::temp_dir().join(format!("rubash2102-{tag}-{}", std::process::id()));
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
        output.status,
    )
}

enum PeerMode {
    /// Greet, record received bytes, stay open.
    Record,
    /// Greet, then close (FIN) immediately.
    GreetThenClose,
    /// Accept, wait for the client's first write to land, then close
    /// WITHOUT reading it — the close carries unread data, so the
    /// peer answers with RST.
    RstOnUnread,
}

struct Listener {
    port: u16,
    rx: mpsc::Receiver<(usize, Vec<u8>)>,
}

fn start_listener(mode: PeerMode) -> Listener {
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
                    match mode {
                        PeerMode::Record => {
                            let _ = stream.write_all(b"GREETING-LINE\n");
                            let _ = stream.flush();
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
                        PeerMode::GreetThenClose => {
                            let _ = stream.write_all(b"GREETING-LINE\n");
                            let _ = stream.flush();
                            // drop: FIN close with nothing unread.
                        }
                        PeerMode::RstOnUnread => {
                            std::thread::sleep(Duration::from_millis(400));
                            // drop without reading: RST.
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

    fn target(&self) -> String {
        format!("/dev/tcp/127.0.0.1/{}", self.port)
    }
}

/// Live-peer delivery through every classified flush face: the bytes
/// must ARRIVE (on Windows these all failed EINVAL before the fix).
#[test]
fn issue2102_live_peer_receives_builtin_and_cat_writes() {
    let listener = start_listener(PeerMode::Record);
    let t = listener.target();
    let (stdout, stderr, status) =
        run_script("live-direct", &format!("echo hi > {t}\necho \"rc=$?\"\n"));
    assert_eq!(status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n");
    let (conns, received) = listener.stats();
    assert_eq!(conns, 1);
    assert_eq!(received, b"hi\n", "direct `>` flush reaches the peer");

    let listener = start_listener(PeerMode::Record);
    let t = listener.target();
    let (stdout, stderr, status) =
        run_script("live-append", &format!("echo hi >> {t}\necho \"rc=$?\"\n"));
    assert_eq!(status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n");
    let (conns, received) = listener.stats();
    assert_eq!(conns, 1);
    assert_eq!(received, b"hi\n", "append `>>` flush reaches the peer");

    let listener = start_listener(PeerMode::Record);
    let t = listener.target();
    let (stdout, stderr, status) = run_script(
        "live-cat",
        &format!("printf 'a\\nb\\n' > data.txt; cat data.txt > {t}\necho \"rc=$?\"\n"),
    );
    assert_eq!(status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n");
    let (conns, received) = listener.stats();
    assert_eq!(conns, 1);
    assert_eq!(received, b"a\nb\n", "emulated cat flush reaches the peer");

    let listener = start_listener(PeerMode::Record);
    let t = listener.target();
    let (stdout, stderr, status) = run_script(
        "live-exec",
        &format!("exec 3<>{t}\necho hi >&3\necho \"rc=$?\"\n"),
    );
    assert_eq!(status.code(), Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=0\n");
    let (conns, received) = listener.stats();
    assert_eq!(conns, 1);
    assert_eq!(received, b"hi\n", "exec-bound fd write reaches the peer");
}

/// FIN-closed peer: greeting read, EOF confirmed, first write
/// succeeds, second write hits the dead peer. Never EINVAL.
#[test]
fn issue2102_fin_closed_peer_write_error_family() {
    let listener = start_listener(PeerMode::GreetThenClose);
    let t = listener.target();
    let script = format!(
        "exec 3<>{t}\n\
         read x <&3\n\
         read -t 1 y <&3\n\
         echo \"eof-rc=$?\"\n\
         echo one >&3\n\
         echo \"w1=$?\"\n\
         echo two >&3\n\
         echo \"w2=$?\"\n"
    );
    let (stdout, stderr, status) = run_script("fin", &script);
    assert!(
        stdout.contains("eof-rc=1"),
        "peer FIN reached the reader: stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("w1=0"),
        "first write after FIN succeeds (GNU parity): {stdout}"
    );
    assert!(
        !stderr.contains("Invalid argument"),
        "the #2102 symptom must never appear: {stderr}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.signal(),
            Some(13),
            "second write raises SIGPIPE and the shell dies, like GNU: \
             stdout={stdout} stderr={stderr}"
        );
    }
    #[cfg(windows)]
    {
        assert_eq!(status.code(), Some(0), "script completes: {stderr}");
        assert!(
            stdout.contains("w2=1"),
            "second write fails the builtin: {stdout}"
        );
        assert!(
            stderr.contains("write error:"),
            "sh_chkwrite diagnostic present: {stderr}"
        );
    }
}

/// RST-closed peer (close with the client's write unread): the next
/// write fails with the reset/broken-pipe family, never EINVAL.
#[test]
fn issue2102_rst_closed_peer_write_error_family() {
    let listener = start_listener(PeerMode::RstOnUnread);
    let t = listener.target();
    let script = format!(
        "exec 3<>{t}\n\
         echo one >&3\n\
         echo \"w1=$?\"\n\
         read -t 2 x <&3\n\
         echo two >&3\n\
         echo \"w2=$?\"\n"
    );
    let (stdout, stderr, status) = run_script("rst", &script);
    assert!(
        stdout.contains("w1=0"),
        "first write lands before the RST: {stdout} stderr={stderr}"
    );
    assert!(
        !stderr.contains("Invalid argument"),
        "the #2102 symptom must never appear: {stderr}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let died_by_sigpipe = status.signal() == Some(13);
        let reported = stdout.contains("w2=1") && stderr.contains("write error:");
        assert!(
            died_by_sigpipe || reported,
            "second write dies by SIGPIPE or reports the write error \
             (GNU shows one of the two by timing): stdout={stdout} stderr={stderr}"
        );
        if reported {
            assert!(
                stderr.contains("Broken pipe") || stderr.contains("Connection reset by peer"),
                "error text is the EPIPE family: {stderr}"
            );
        }
    }
    #[cfg(windows)]
    {
        assert_eq!(status.code(), Some(0), "script completes: {stderr}");
        assert!(
            stdout.contains("w2=1"),
            "second write fails the builtin: {stdout}"
        );
        assert!(
            stderr.contains("write error:"),
            "sh_chkwrite diagnostic present: {stderr}"
        );
    }
    let _ = status;
}
