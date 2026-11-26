//! rubash#2098 — a non-winning fd-0 net redirect never executed its
//! open: GNU applies EVERY redirect's open in order (redir.c
//! do_redirections); only the last fd-0 binding survives, but a
//! superseded `/dev/tcp` entry still connects exactly once, and a
//! refused superseded connect fails the command with the two-line
//! diagnostic. The #1950 winner-probe purity design validated net
//! targets without connecting and let only the winner's bind connect,
//! so a losing net entry produced ZERO connections (and a refused
//! loser silently ran the command). The loser's open is now settled
//! in the common reject gate (`reject_invalid_redirects`), the one
//! walk every funnel (spawn / emulated / builtin / pipeline stage)
//! passes exactly once; the winner stays probe-pure.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! loopback listener counting connections, 2026-10-29): every shape
//! below connects exactly the GNU count with byte-equal output.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash2098-{tag}-{}", std::process::id()));
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

/// Loopback listener: greets each accepted connection and reports the
/// running connection count (the #2091 harness shape).
struct Listener {
    port: u16,
    rx: mpsc::Receiver<usize>,
}

fn start_listener() -> Listener {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking");
        let mut count = 0usize;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    count += 1;
                    let _ = stream.write_all(b"GREETING-LINE\n");
                    let _ = stream.flush();
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
                    let mut sink = [0u8; 64];
                    loop {
                        match stream.read(&mut sink) {
                            Ok(0) => break,
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
            if tx.send(count).is_err() {
                break;
            }
        }
    });
    Listener { port, rx }
}

impl Listener {
    fn connections(&self) -> usize {
        std::thread::sleep(Duration::from_millis(500));
        let mut last = 0;
        while let Ok(count) = self.rx.try_recv() {
            last = count;
        }
        last
    }

    fn target(&self) -> String {
        format!("/dev/tcp/127.0.0.1/{}", self.port)
    }
}

fn assert_shape(tag: &str, script: &str, want_out: &str) {
    let (stdout, stderr, code) = run_script(tag, script);
    assert_eq!(code, Some(0), "{tag}: exit code (stderr: {stderr})");
    assert_eq!(stdout, want_out, "{tag}: stdout");
}

/// Winner controls: exactly one connection, greeting served.
#[test]
fn issue2098_winner_controls_connect_once() {
    let listener = start_listener();
    let t = listener.target();
    assert_shape("win-ext", &format!("/usr/bin/head -c 5 < {t}\n"), "GREET");
    assert_eq!(listener.connections(), 1, "winner Input (spawn funnel)");

    let listener = start_listener();
    let t = listener.target();
    assert_shape("win-cat", &format!("cat < {t} | head -c 5\n"), "GREET");
    assert_eq!(listener.connections(), 1, "winner Input (emulated cat)");

    let listener = start_listener();
    let t = listener.target();
    assert_shape(
        "win-rw",
        &format!("dd bs=5 count=1 2>/dev/null <> {t}\n"),
        "GREET",
    );
    assert_eq!(listener.connections(), 1, "winner ReadWrite");
}

/// The #2098 core: a net entry superseded by a heredoc/herestring
/// still connects exactly once, on every funnel; the body wins fd 0.
#[test]
fn issue2098_loser_against_body_connects_once() {
    let listener = start_listener();
    let t = listener.target();
    assert_shape(
        "lose-ext",
        &format!("/usr/bin/head < {t} <<EOF\nBODY\nEOF\n"),
        "BODY\n",
    );
    assert_eq!(listener.connections(), 1, "Input loser vs heredoc (spawn)");

    let listener = start_listener();
    let t = listener.target();
    assert_shape(
        "lose-cat",
        &format!("cat < {t} <<EOF\nBODY\nEOF\n"),
        "BODY\n",
    );
    assert_eq!(
        listener.connections(),
        1,
        "Input loser vs heredoc (emulated)"
    );

    let listener = start_listener();
    let t = listener.target();
    assert_shape(
        "lose-rw-cat",
        &format!("cat <> {t} <<EOF\nBODY\nEOF\n"),
        "BODY\n",
    );
    assert_eq!(listener.connections(), 1, "ReadWrite loser vs heredoc");

    let listener = start_listener();
    let t = listener.target();
    assert_shape(
        "lose-rw-ext",
        &format!("/usr/bin/head <> {t} <<<BODY\n"),
        "BODY\n",
    );
    assert_eq!(
        listener.connections(),
        1,
        "ReadWrite loser vs herestring (spawn)"
    );

    let listener = start_listener();
    let t = listener.target();
    assert_shape(
        "lose-builtin",
        &format!("read x < {t} <<<BODY; echo \"got=$x\"\n"),
        "got=BODY\n",
    );
    assert_eq!(
        listener.connections(),
        1,
        "Input loser vs herestring (builtin)"
    );
}

/// Losers against non-body winners, doubles, pipeline stages, close.
#[test]
fn issue2098_loser_against_file_and_double_and_pipeline() {
    let listener = start_listener();
    let t = listener.target();
    assert_shape(
        "lose-file",
        &format!("printf FILEDATA > data.txt; cat < {t} < data.txt\n"),
        "FILEDATA",
    );
    assert_eq!(listener.connections(), 1, "Input loser vs later file Input");

    let listener = start_listener();
    let t = listener.target();
    assert_shape("double", &format!("cat < {t} < {t} | head -c 5\n"), "GREET");
    assert_eq!(
        listener.connections(),
        2,
        "two written net entries connect twice"
    );

    let listener = start_listener();
    let t = listener.target();
    assert_shape(
        "lose-pipe",
        &format!("cat < {t} <<EOF | tr a-z A-Z\nbody\nEOF\n"),
        "BODY\n",
    );
    assert_eq!(listener.connections(), 1, "Input loser in a pipeline stage");

    let listener = start_listener();
    let t = listener.target();
    let (stdout, _stderr, code) = run_script(
        "lose-close",
        &format!("cat < {t} <&-; echo \"inner-rc=$?\"\n"),
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "inner-rc=1\n", "fd 0 closed after the net open");
    assert_eq!(listener.connections(), 1, "Input loser vs CloseInput");
}

/// A refused superseded connect fails the command: the heredoc body
/// is NOT served, the status is 1, and stderr carries GNU's two lines
/// (netopen's `connect:` pre-line, then redirection_error's target
/// line).
#[test]
fn issue2098_refused_loser_fails_command() {
    // Bind then drop a listener so connects to its port are refused.
    let refused_port = {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let p = l.local_addr().expect("addr").port();
        drop(l);
        p
    };
    let (stdout, stderr, code) = run_script(
        "refused",
        &format!("cat < /dev/tcp/127.0.0.1/{refused_port} <<EOF\nBODY\nEOF\necho \"cat-rc=$?\"\n"),
    );
    assert_eq!(code, Some(0), "script continues after the failed command");
    assert_eq!(stdout, "cat-rc=1\n", "command failed, body not served");
    assert!(
        stderr.contains("connect: Connection refused"),
        "netopen pre-line present: {stderr}"
    );
    assert!(
        stderr.contains(&format!(
            "/dev/tcp/127.0.0.1/{refused_port}: Connection refused"
        )),
        "redirection_error target line present: {stderr}"
    );
}
