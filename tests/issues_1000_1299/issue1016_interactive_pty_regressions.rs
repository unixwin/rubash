//! rubash#1016 — interactive mode driven through a `script(1)` PTY
//! dropped every input line after the first (TERM=dumb: rest lost,
//! clean exit; xterm: hang). Closed in the zero-queue campaign on
//! desktop evidence: under `script -qec '<shell> -i'` with paced
//! input, both commands execute and the shell exits cleanly, matching
//! GNU. The defect lived in the host terminal layer of the original
//! report environment; this pin keeps the engine-side interactive
//! reader honest against the same harness.
//!
//! Markers are computed values (`A42Z`, `B123Z`) that never appear in
//! the echoed input lines, so a pass requires real execution of both
//! lines — not just terminal echo.

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn interactive_session_consumes_all_paced_lines_under_script_pty() {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("no script(1) on this host; skipping");
        return;
    }
    let cmdline = format!("{} -i", env!("CARGO_BIN_EXE_rubash"));
    let mut child = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .env("TERM", "dumb")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn script(1)");

    let mut stdin = child.stdin.take().expect("child stdin");
    let writer = std::thread::spawn(move || {
        for line in ["echo A$((6*7))Z\n", "echo B$((100+23))Z\n", "exit\n"] {
            if stdin.write_all(line.as_bytes()).is_err() {
                break;
            }
            let _ = stdin.flush();
            std::thread::sleep(Duration::from_secs(1));
        }
    });

    let mut stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    let buf = match rx.recv_timeout(Duration::from_secs(25)) {
        Ok(buf) => buf,
        Err(_) => {
            let _ = child.kill();
            panic!(
                "interactive session did not finish within 25s (input lines lost or shell hung)"
            );
        }
    };
    let status = child.wait().expect("wait for script(1)");
    let _ = writer.join();
    let _ = reader.join();

    let text = String::from_utf8_lossy(&buf).into_owned();
    assert!(
        text.contains("A42Z"),
        "first command output missing from session transcript: {text:?}"
    );
    assert!(
        text.contains("B123Z"),
        "second command output missing from session transcript: {text:?}"
    );
    assert_eq!(status.code(), Some(0), "transcript: {text:?}");
}
