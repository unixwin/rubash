//! rubash#2072 — an interactive session's background (`&`) child is a
//! respawned `rubash -c` process carrying every function definition as
//! serialized text. The child inherited the parent's history/histexpand
//! option state AND re-ran the interactive startup defaults, so it
//! history-expanded its own transport text: streams of
//! `!x: event not found`, blanked lines corrupting the parse (the
//! `nvm ls` noise), a parent's `set +H` silently undone inside the
//! child, and transport fragments recorded into $HISTFILE at child
//! exit. GNU's background child is a fork (execute_cmd.c
//! execute_in_subshell): option state is inherited verbatim, no
//! startup runs, and bashhist.c pre_process_line never applies to its
//! already-parsed commands.
//!
//! These pins drive the real binary interactively (PTY via script(1),
//! and `-i` over pipes) with functions whose bodies carry `!` tokens in
//! the shapes real scripts carry them (shell negation, a multi-line
//! single-quoted awk program with `!~` / `!n`).

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2072-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create sandbox dir");
    dir
}

fn write_defs(home: &PathBuf) -> PathBuf {
    let defs = home.join("defs2072.sh");
    fs::write(
        &defs,
        "f2072() { if ! [ -e /no/such/path2072 ]; then echo F2072-RAN; fi; }\n\
         g2072() { command awk '\n\
         BEGIN { n = 0 }\n\
         $1 !~ /^zz/ { print \"G2072:\" $1; n++ }\n\
         END { if (!n) print \"G2072-EMPTY\" }\n\
         '; }\n\
         v2072() { set -o | grep histexpand; }\n",
    )
    .expect("write defs");
    let input = home.join("input2072.txt");
    fs::write(&input, "apple\nzz-top\n").expect("write input");
    defs
}

/// Spawn `rubash -i` with piped stdio in a sandboxed HOME, feed `lines`,
/// and return the combined stdout+stderr text.
fn run_pipe_session(home: &PathBuf, lines: &[String], timeout_s: u64) -> String {
    let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    let mut stdin = child.stdin.take().expect("child stdin");
    let script = lines.join("\n") + "\n";
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(script.as_bytes());
        let _ = stdin.flush();
        // Close stdin so the shell sees EOF after the scripted lines.
        drop(stdin);
    });
    let (tx, rx) = mpsc::channel();
    let mut out = child.stdout.take().expect("child stdout");
    let mut err = child.stderr.take().expect("child stderr");
    let t_out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let (tx2, rx2) = mpsc::channel();
    let t_err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        let _ = tx2.send(buf);
    });
    let deadline = Instant::now() + Duration::from_secs(timeout_s);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait failed: {e}"),
        }
    };
    let _ = writer.join();
    let stdout = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let stderr = rx2.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = t_out.join();
    let _ = t_err.join();
    assert!(
        status.is_some_and(|s| s.success()),
        "interactive session did not exit cleanly: {status:?}"
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    )
}

#[test]
fn background_child_does_not_history_expand_its_transport_text() {
    let home = sandbox("pipe");
    let defs = write_defs(&home);
    let text = run_pipe_session(
        &home,
        &[
            format!("source {}", defs.display()),
            "f2072 & wait".to_string(),
            "echo TAIL2072".to_string(),
            "exit".to_string(),
        ],
        60,
    );
    assert!(
        text.contains("F2072-RAN"),
        "background function never ran (transport parse damaged?): {text:?}"
    );
    assert!(
        text.contains("TAIL2072"),
        "session did not continue past the background job: {text:?}"
    );
    assert!(
        !text.contains("event not found"),
        "respawned child history-expanded its transport text: {text:?}"
    );
    assert!(
        !text.contains("syntax error"),
        "respawned child parse was damaged by expansion: {text:?}"
    );
}

#[test]
fn background_child_inherits_histexpand_state_verbatim() {
    let home = sandbox("opts");
    let defs = write_defs(&home);
    let text = run_pipe_session(
        &home,
        &[
            format!("source {}", defs.display()),
            "v2072 & wait".to_string(),
            "set +H".to_string(),
            "v2072 & wait".to_string(),
            "exit".to_string(),
        ],
        60,
    );
    let squashed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        squashed.contains("histexpand on"),
        "default interactive child must report histexpand on: {text:?}"
    );
    assert!(
        squashed.contains("histexpand off"),
        "child must inherit the parent's `set +H` verbatim (GNU fork \
         inheritance), not re-force the interactive default: {text:?}"
    );
}

#[test]
fn interactive_typed_history_expansion_still_works() {
    let home = sandbox("typed");
    let text = run_pipe_session(
        &home,
        &[
            "echo H$((20+22))Z".to_string(),
            "!!".to_string(),
            "exit".to_string(),
        ],
        60,
    );
    // The computed marker appears once from the original line and once
    // from the `!!` re-execution; typed input echo carries the literal
    // text, never the computed value.
    assert_eq!(
        text.matches("H42Z").count(),
        2,
        "typed `!!` must still expand against the session history: {text:?}"
    );
}

#[test]
fn interactive_pty_background_child_is_clean_and_history_file_untouched() {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("no script(1) on this host; skipping");
        return;
    }
    let home = sandbox("pty");
    let defs = write_defs(&home);
    let input = home.join("input2072.txt");
    let cmdline = format!("{} -i", env!("CARGO_BIN_EXE_rubash"));
    let mut child = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .env_clear()
        .env("HOME", &home)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("TERM", "dumb")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn script(1)");

    let mut stdin = child.stdin.take().expect("child stdin");
    let lines = [
        format!("source {}", defs.display()),
        "f2072 & wait".to_string(),
        format!("g2072 < {} & wait", input.display()),
        "exit".to_string(),
    ];
    let writer = std::thread::spawn(move || {
        for line in &lines {
            if stdin.write_all(format!("{line}\n").as_bytes()).is_err() {
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

    let buf = match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(buf) => buf,
        Err(_) => {
            let _ = child.kill();
            panic!("interactive PTY session did not finish within 30s");
        }
    };
    let status = child.wait().expect("wait for script(1)");
    let _ = writer.join();
    let _ = reader.join();

    let text = String::from_utf8_lossy(&buf).into_owned();
    assert!(
        text.contains("F2072-RAN"),
        "background function output missing: {text:?}"
    );
    assert!(
        text.contains("G2072:apple"),
        "background awk function output missing: {text:?}"
    );
    assert!(
        !text.contains("event not found"),
        "respawned child history-expanded its transport text: {text:?}"
    );
    assert!(
        !text.contains("syntax error"),
        "respawned child parse was damaged by expansion: {text:?}"
    );
    assert_eq!(status.code(), Some(0), "transcript: {text:?}");

    // The history file is written by the top-level shell only, and holds
    // the typed lines — never fragments of the serialized transport.
    let histfile = home.join(".bash_history");
    if histfile.exists() {
        let hist = fs::read_to_string(&histfile).expect("read history file");
        assert!(
            !hist.contains("BEGIN") && !hist.contains("G2072:"),
            "history file polluted with respawn transport fragments: {hist:?}"
        );
    }
}
