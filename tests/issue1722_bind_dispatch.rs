//! rubash#1722 — the standalone interactive reader never dispatched
//! `bind` key bindings on a real terminal: the Unix tty stayed in
//! canonical mode (the kernel line discipline owned editing), and the
//! edit dispatcher never consulted the bind registry, so `bind -x`
//! commands could not fire per keystroke and READLINE_LINE /
//! READLINE_POINT were never exchanged. Frameworks built on those
//! bindings (ble.sh) could not take over input — ble's terminal
//! cursor-position replies were read back as command text.
//!
//! Pins drive `rubash -i` through a `script(1)` PTY with paced input
//! (the repository's PTY harness form, see issue1016 pins) and assert
//! the GNU bash 5.3 outcomes, hand-measured against the GNU oracle in
//! this environment: a bound key fires mid-line, sees the current
//! buffer and point, and its READLINE_LINE rewrite is what executes.
//! Markers are computed values that never appear in typed text, so a
//! pass requires real execution of the mutated line.

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Run one paced PTY session: each chunk is written, flushed, and
/// given time to be processed before the next arrives. Returns the
/// full session transcript.
fn run_pty_session(chunks: &[&[u8]]) -> String {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("no script(1) on this host; skipping");
        return String::new();
    }
    let home = std::env::temp_dir().join(format!("rubash-1722-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("create scratch home");
    std::fs::write(home.join(".bashrc"), "").expect("write empty bashrc");
    let cmdline = format!("{} -i", env!("CARGO_BIN_EXE_rubash"));
    let mut child = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn script(1)");

    let chunks: Vec<Vec<u8>> = chunks.iter().map(|c| c.to_vec()).collect();
    let mut stdin = child.stdin.take().expect("child stdin");
    let writer = std::thread::spawn(move || {
        // Let the shell reach its first prompt before typing.
        std::thread::sleep(Duration::from_millis(1200));
        for chunk in &chunks {
            if stdin.write_all(chunk).is_err() {
                break;
            }
            let _ = stdin.flush();
            std::thread::sleep(Duration::from_millis(900));
        }
    });

    let mut stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    let buf = match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(buf) => buf,
        Err(_) => {
            let _ = child.kill();
            panic!("interactive session did not finish within 60s (shell hung)");
        }
    };
    let _ = child.wait();
    let _ = writer.join();
    let _ = reader.join();
    let _ = std::fs::remove_dir_all(&home);
    String::from_utf8_lossy(&buf).into_owned()
}

#[test]
fn issue1722_bind_x_fires_mid_line_and_mutates_the_buffer() {
    let text = run_pty_session(&[
        b"f() { echo \"RL=[$READLINE_LINE] P=$READLINE_POINT\"; READLINE_LINE='echo MUTATED'; READLINE_POINT=12; }\n",
        b"bind -x '\"\\C-t\": f'\n",
        b"echo orig",
        b"\x14",
        b"\n",
        b"exit\n",
    ]);
    if text.is_empty() {
        return; // no script(1) on this host
    }
    assert!(
        text.contains("RL=[echo orig] P=9"),
        "bind -x command did not see the live buffer/point: {text:?}"
    );
    assert!(
        text.contains("\r\nMUTATED\r\n") || text.contains("\nMUTATED\r\n"),
        "the READLINE_LINE rewrite was not what executed: {text:?}"
    );
    assert!(
        !text.contains("\r\norig\r\n"),
        "the unmutated line executed as well: {text:?}"
    );
}

#[test]
fn issue1722_function_binding_and_macro_binding_dispatch() {
    let text = run_pty_session(&[
        // C-g rebound from its default (abort) to unix-line-discard.
        b"bind '\"\\C-g\": unix-line-discard'\n",
        // A two-key macro: C-x C-l inserts command text as if typed.
        b"bind '\"\\C-x\\C-l\": \"echo M$((40+2))\"'\n",
        b"echo GONE$((5+5))",
        b"\x07",
        b"\n",
        b"\x18\x0c",
        b"\n",
        b"exit\n",
    ]);
    if text.is_empty() {
        return; // no script(1) on this host
    }
    assert!(
        !text.contains("GONE10"),
        "unix-line-discard binding did not discard the line: {text:?}"
    );
    assert!(
        text.contains("M42"),
        "macro binding did not insert/execute its text: {text:?}"
    );
}
