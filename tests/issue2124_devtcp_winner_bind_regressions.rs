//! rubash#2124 — a WINNING /dev/tcp input redirect on an in-process
//! command (builtin, function call, compound) never connected and
//! never errored: silent no-op (`echo hi < /dev/tcp/...` connected
//! zero times and reported success; a refused connect also ran the
//! body with rc 0). GNU applies EVERY redirect's open before the
//! body runs, for every command class (redir.c do_redirections;
//! redir_open -> netopen for /dev/tcp|udp targets): the winner's
//! open IS the connect, exactly once, and a refused connect fails
//! the command with the two-line diagnostic before the body runs.
//! rubash's in-process funnels only PROBED the winner (the #1950
//! validate-by-resolution rule: a probe must not connect, the bind
//! performs the one open) but had no bind. The bind now exists at
//! the shared layers: the simple-command funnel binds the fd-0
//! winner and surviving numbered net inputs for builtin/function
//! routes, the compound fd-0 bind and the function-call stdin
//! classification connect net targets, and the numbered-fd ladder
//! connects net entries — while fd-0 LOSERS stay settled by the
//! reject gate (rubash#2098) and external routes keep binding in
//! spawn setup, so every shape connects exactly the GNU count.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! loopback listener counting connections, 2026-10-29).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let (tx, rx) = mpsc::channel();
    let script = script.to_string();
    let panic_tag = tag.to_string();
    let tag = tag.to_string();
    std::thread::spawn(move || {
        let dir = std::env::temp_dir().join(format!("rubash2124-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let case = dir.join("case.sh");
        std::fs::write(&case, &script).expect("write fixture");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .current_dir(&dir)
            .arg(&case)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run rubash");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = tx.send((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        ));
    });
    rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| panic!("{panic_tag}: rubash run timed out"))
}

/// Loopback listener: greets each accepted connection and reports the
/// running connection count (the #2098 harness shape).
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
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
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

    /// Poll until the count reaches `want` (3s deadline) — the
    /// accept thread's reports can lag under parallel test load, so
    /// a positive expectation waits instead of sampling once.
    fn connections_expect(&self, want: usize) -> usize {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut last = 0;
        loop {
            while let Ok(count) = self.rx.try_recv() {
                last = count;
            }
            if last >= want || std::time::Instant::now() >= deadline {
                return last;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn target(&self) -> String {
        format!("/dev/tcp/127.0.0.1/{}", self.port)
    }
}

/// A port nothing listens on: bind, learn the port, drop.
fn closed_target() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    format!("/dev/tcp/127.0.0.1/{port}")
}

/// Builtin winner: connects exactly once even though the body never
/// reads (GNU opens the redirect before running the command).
#[test]
fn issue2124_builtin_winner_connects_once() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script("echo-win", &format!("echo hi < {t}\n"));
    assert_eq!(code, Some(0), "echo winner: exit code (stderr: {stderr})");
    assert_eq!(stdout, "hi\n", "echo winner: stdout");
    assert_eq!(
        listener.connections_expect(1),
        1,
        "echo winner: connections"
    );

    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script("printf-win", &format!("printf 'p\\n' < {t}\n"));
    assert_eq!(code, Some(0), "printf winner: exit code (stderr: {stderr})");
    assert_eq!(stdout, "p\n", "printf winner: stdout");
    assert_eq!(
        listener.connections_expect(1),
        1,
        "printf winner: connections"
    );
}

/// A reading builtin: the bound socket is what `read` consumes — one
/// connection total (the funnel bind's), the greeting is the data.
#[test]
fn issue2124_read_winner_connects_once_and_reads() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) =
        run_script("read-win", &format!("read x < {t}\necho \"[${{x}}]\"\n"));
    assert_eq!(code, Some(0), "read winner: exit code (stderr: {stderr})");
    assert_eq!(stdout, "[GREETING-LINE]\n", "read winner: stdout");
    assert_eq!(
        listener.connections_expect(1),
        1,
        "read winner: connections"
    );

    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script(
        "mapfile-win",
        &format!("mapfile -t a < {t}\necho \"[${{a[0]}}]\"\n"),
    );
    assert_eq!(
        code,
        Some(0),
        "mapfile winner: exit code (stderr: {stderr})"
    );
    assert_eq!(stdout, "[GREETING-LINE]\n", "mapfile winner: stdout");
    assert_eq!(
        listener.connections_expect(1),
        1,
        "mapfile winner: connections"
    );
}

/// Function call and compound winners: the call/group redirect binds
/// on fd 0 for the body's duration.
#[test]
fn issue2124_function_and_compound_winners_connect_once() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script(
        "func-win",
        &format!("f() {{ read x; echo \"[${{x}}]\"; }}\nf < {t}\n"),
    );
    assert_eq!(
        code,
        Some(0),
        "function winner: exit code (stderr: {stderr})"
    );
    assert_eq!(stdout, "[GREETING-LINE]\n", "function winner: stdout");
    assert_eq!(
        listener.connections_expect(1),
        1,
        "function winner: connections"
    );

    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script(
        "brace-win",
        &format!("{{ read x; echo \"[${{x}}]\"; }} < {t}\n"),
    );
    assert_eq!(code, Some(0), "brace winner: exit code (stderr: {stderr})");
    assert_eq!(stdout, "[GREETING-LINE]\n", "brace winner: stdout");
    assert_eq!(
        listener.connections_expect(1),
        1,
        "brace winner: connections"
    );

    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script(
        "loop-win",
        &format!("while read x; do echo \"[${{x}}]\"; done < {t}\n"),
    );
    assert_eq!(code, Some(0), "loop winner: exit code (stderr: {stderr})");
    assert_eq!(stdout, "[GREETING-LINE]\n", "loop winner: stdout");
    assert_eq!(
        listener.connections_expect(1),
        1,
        "loop winner: connections"
    );

    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script("subshell-win", &format!("(cat) < {t}\n"));
    assert_eq!(
        code,
        Some(0),
        "subshell winner: exit code (stderr: {stderr})"
    );
    assert_eq!(stdout, "GREETING-LINE\n", "subshell winner: stdout");
    assert_eq!(
        listener.connections_expect(1),
        1,
        "subshell winner: connections"
    );
}

/// Numbered-fd winner on a simple builtin: `3< /dev/tcp/...` connects
/// once and `read -u 3` consumes the greeting.
#[test]
fn issue2124_numbered_winner_connects_once() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script(
        "fd3-win",
        &format!("read -u 3 x 3< {t}\necho \"[${{x}}]\"\n"),
    );
    assert_eq!(code, Some(0), "fd3 winner: exit code (stderr: {stderr})");
    assert_eq!(stdout, "[GREETING-LINE]\n", "fd3 winner: stdout");
    assert_eq!(listener.connections_expect(1), 1, "fd3 winner: connections");

    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script("pipe-echo-win", &format!("echo hi < {t} | cat\n"));
    assert_eq!(
        code,
        Some(0),
        "pipeline stage: exit code (stderr: {stderr})"
    );
    assert_eq!(stdout, "hi\n", "pipeline stage: stdout");
    assert_eq!(
        listener.connections_expect(1),
        1,
        "pipeline stage: connections"
    );
}

/// Loser + winner on one reading builtin: the gate settles the loser
/// (one connect), the bind connects the winner (one connect), and the
/// data comes from the winner.
#[test]
fn issue2124_loser_and_winner_each_connect_once() {
    let loser = start_listener();
    let winner = start_listener();
    let lt = loser.target();
    let wt = winner.target();
    let (stdout, stderr, code) = run_script(
        "loser-winner",
        &format!("read x < {lt} < {wt}\necho \"[${{x}}]\"\n"),
    );
    assert_eq!(code, Some(0), "loser+winner: exit code (stderr: {stderr})");
    assert_eq!(stdout, "[GREETING-LINE]\n", "loser+winner: stdout");
    assert_eq!(loser.connections_expect(1), 1, "loser: connections");
    assert_eq!(winner.connections_expect(1), 1, "winner: connections");
}

/// Refused winner: the two-line GNU diagnostic (connect pre-line +
/// `<target>: Connection refused`), nonzero status, body never runs.
#[test]
fn issue2124_refused_winner_fails_before_body() {
    let t = closed_target();
    let (stdout, stderr, code) = run_script("echo-closed", &format!("echo hi < {t}\n"));
    assert_eq!(code, Some(1), "refused echo: exit code");
    assert_eq!(stdout, "", "refused echo: body must not run");
    assert!(
        stderr.contains("Connection refused"),
        "refused echo: diagnostic (stderr: {stderr})"
    );
    assert!(
        stderr.contains(&t),
        "refused echo: diagnostic names the target (stderr: {stderr})"
    );

    let t = closed_target();
    let (stdout, stderr, code) =
        run_script("brace-closed", &format!("{{ read x; echo ran; }} < {t}\n"));
    assert_eq!(code, Some(1), "refused brace: exit code");
    assert_eq!(stdout, "", "refused brace: body must not run");
    assert!(
        stderr.contains("Connection refused"),
        "refused brace: diagnostic (stderr: {stderr})"
    );
}

/// ReadWrite (`<>`) winners: GNU's r_input_output open connects
/// once and binds the one socket on both endpoints of the fd — for
/// a non-reading builtin, a brace group, and a numbered fd alike.
#[test]
fn issue2124_readwrite_winner_connects_once() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script("echo-rw", &format!("echo hi <> {t}\n"));
    assert_eq!(code, Some(0), "echo <>: exit code (stderr: {stderr})");
    assert_eq!(stdout, "hi\n", "echo <>: stdout");
    assert_eq!(listener.connections_expect(1), 1, "echo <>: connections");

    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script(
        "brace-rw",
        &format!("{{ read x; echo \"[${{x}}]\"; }} <> {t}\n"),
    );
    assert_eq!(code, Some(0), "brace <>: exit code (stderr: {stderr})");
    assert_eq!(stdout, "[GREETING-LINE]\n", "brace <>: stdout");
    assert_eq!(listener.connections_expect(1), 1, "brace <>: connections");

    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script(
        "fd3-rw",
        &format!("read -u 3 x 3<> {t}\necho \"[${{x}}]\"\n"),
    );
    assert_eq!(code, Some(0), "fd3 <>: exit code (stderr: {stderr})");
    assert_eq!(stdout, "[GREETING-LINE]\n", "fd3 <>: stdout");
    assert_eq!(listener.connections_expect(1), 1, "fd3 <>: connections");

    let t = closed_target();
    let (stdout, stderr, code) = run_script("echo-rw-closed", &format!("echo hi <> {t}\n"));
    assert_eq!(code, Some(1), "refused <>: exit code");
    assert_eq!(stdout, "", "refused <>: body must not run");
    assert!(
        stderr.contains("Connection refused"),
        "refused <>: diagnostic (stderr: {stderr})"
    );
}

/// External winner control: spawn setup still performs the one
/// connect — the in-process bind must not add a second one.
#[test]
fn issue2124_external_winner_still_connects_once() {
    let listener = start_listener();
    let t = listener.target();
    let (stdout, stderr, code) = run_script("ext-win", &format!("/usr/bin/head -c 5 < {t}\n"));
    assert_eq!(
        code,
        Some(0),
        "external winner: exit code (stderr: {stderr})"
    );
    assert_eq!(stdout, "GREET", "external winner: stdout");
    assert_eq!(
        listener.connections_expect(1),
        1,
        "external winner: connections"
    );
}
