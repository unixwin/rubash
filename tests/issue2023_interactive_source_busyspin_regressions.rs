//! rubash#2023 — sourcing a large function-definition file (an
//! nvm.sh-shaped workload: 140 function definitions gathered as ONE
//! file-wide definition group, ~3,370 lines / ~76 KB, ending in
//! top-level registration calls) inside an INTERACTIVE session once
//! took minutes of single-core busy-spin on real hardware, while
//! the same source from `-c` finished in about a second.
//!
//! Mechanism (fixed by #1940 + #1961, pinned here so it cannot come
//! back silently): an interactive session defaults `expand_aliases`
//! on, which routes `source` through the alias-live feeder arm. That
//! arm used to re-run alias expansion plus a full completeness
//! re-judgment over the ACCUMULATED group for every candidate
//! complete line, and `run_source_groups` re-scanned gatherer-
//! certified groups wholesale — quadratic in the sourced file's
//! size. Non-interactive shells never took the arm (aliases off),
//! which is why the lab's `-c` runs could not reproduce the hang.
//!
//! The workload's shape matters and is calibrated, not guessed:
//! the 140 definitions are wrapped in a single outer definition,
//! so the gatherer accumulates one file-wide group — the shape
//! nvm.sh effectively presents (its gather parks early and the
//! group swallows the file). A file of 140 SEPARATE top-level
//! definitions does not exercise the bug at all: each definition
//! certifies as its own small group (measured on the pre-fix
//! binary at the #2023 pin b95d28ae, debug, same machine: 7.9 s,
//! no spin). The wrapped shape on that same pre-fix binary was
//! still inside `source` when killed at 240 s with ~43 CPU-s
//! burned; the real nvm.sh behaved identically (>240 s, ~47 CPU-s
//! at kill; no-alias control completed in 12 s). On current master
//! the identical session finishes in ~1.2 s.
//!
//! These pins drive the real binary interactively (`-i` over pipes,
//! and under a real PTY via script(1) where available) with an alias
//! preset — the exact trigger condition — and bound the wall time.
//! The bounds are deliberately generous (healthy runs finish in
//! seconds even in a debug build on a loaded box); the historical
//! failure shape exceeded 120 s in a RELEASE build on the same
//! workload class, so any reintroduction of the quadratic arm fails
//! here long before it can ship.
//!
//! GNU reference point (lab, same machine, release): GNU bash
//! 5.3.20 sources the real nvm.sh interactively in ~0.3 s; rubash's
//! healthy interactive source of the same file is ~0.3 s of CPU.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2023-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create sandbox dir");
    dir
}

/// Write the nvm.sh-shaped workload: 140 function definitions with
/// case/if/comsub bodies (the constructs nvm.sh's 125 functions are
/// built from), wrapped in ONE outer definition so the source
/// gatherer accumulates a single file-wide group (see the header
/// comment for why the wrapping is the calibrated trigger shape),
/// followed by the top-level registration tail real init scripts
/// end with. Returns the path.
fn write_workload(home: &PathBuf) -> PathBuf {
    let path = home.join("workload2023.sh");
    let mut s = String::new();
    s.push_str("# nvm.sh-shaped workload for rubash#2023 (synthetic)\n");
    s.push_str("w2023_all() {\n");
    for i in 0..140 {
        s.push_str(&format!(
            "w2023_fn_{i}() {{\n\
             \x20 local _v=\"${{1:-}}\"\n\
             \x20 local _kind='other'\n\
             \x20 local _n=0\n\
             \x20 case \"${{_v}}\" in\n\
             \x20   'lts/'*) _kind='lts' ;;\n\
             \x20   'v'[0-9]*) _kind='std' ;;\n\
             \x20   'node'|'iojs') _kind='named' ;;\n\
             \x20   '') _kind='empty' ;;\n\
             \x20   *) _kind='other' ;;\n\
             \x20 esac\n\
             \x20 while [ \"${{_n}}\" -lt 3 ]; do\n\
             \x20   _n=$((_n + 1))\n\
             \x20   if [ \"${{_kind}}\" = 'std' ]; then\n\
             \x20     _kind='std-verified'\n\
             \x20   fi\n\
             \x20 done\n\
             \x20 local _echoed=\"$(printf '%s' \"${{_v}}\")\"\n\
             \x20 if [ -n \"${{_echoed}}\" ] && [ \"${{_kind}}\" != 'empty' ]; then\n\
             \x20   printf '%s:%s\\n' \"${{_kind}}\" \"${{_echoed}}\"\n\
             \x20 else\n\
             \x20   printf 'none\\n'\n\
             \x20 fi\n\
             }}\n"
        ));
    }
    s.push_str("}\n");
    s.push_str("w2023_all\n");
    s.push_str("W2023_LOADED=1\n");
    s.push_str("w2023_fn_7 'v0.0.0' > /dev/null\n");
    s.push_str("complete -F _w2023_noop w2023 2>/dev/null || true\n");
    s.push_str("_w2023_noop() { :; }\n");
    fs::write(&path, s).expect("write workload");
    path
}

/// Spawn `rubash -i` with piped stdio in a sandboxed HOME, feed
/// `lines`, and return (combined output, elapsed). Panics via the
/// caller's deadline discipline if the session does not exit in
/// `timeout_s`.
fn run_pipe_session(home: &PathBuf, lines: &[String], timeout_s: u64) -> (String, Duration) {
    let started = Instant::now();
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
    let elapsed = started.elapsed();
    let _ = writer.join();
    let stdout = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let stderr = rx2.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = t_out.join();
    let _ = t_err.join();
    assert!(
        status.is_some_and(|s| s.success()),
        "interactive session did not exit cleanly within {timeout_s}s: {status:?}"
    );
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        ),
        elapsed,
    )
}

fn session_lines(workload: &PathBuf) -> Vec<String> {
    vec![
        // The trigger condition for the historical spin: an alias is
        // live in an interactive (expand_aliases-on) session while a
        // large definition file is sourced.
        "alias ll='ls -l'".to_string(),
        format!("source {}", workload.display()),
        "echo SRC2023-RC-$?".to_string(),
        "w2023_fn_7 'v9.9'".to_string(),
        "echo W2023-LOADED-${W2023_LOADED:-unset}".to_string(),
        "exit".to_string(),
    ]
}

fn assert_clean_load(text: &str) {
    assert!(
        text.contains("SRC2023-RC-0"),
        "sourcing the workload did not complete with rc=0: {text:?}"
    );
    assert!(
        text.contains("std-verified:v9.9"),
        "a workload function did not run correctly after the load: {text:?}"
    );
    assert!(
        text.contains("W2023-LOADED-1"),
        "the workload's top-level tail did not execute: {text:?}"
    );
    assert!(
        !text.contains("syntax error"),
        "interactive load produced syntax noise: {text:?}"
    );
    assert!(
        !text.contains("event not found"),
        "interactive load produced history-expansion noise: {text:?}"
    );
}

#[test]
fn interactive_pipe_source_of_large_definition_file_is_bounded() {
    let home = sandbox("pipe");
    let workload = write_workload(&home);
    let (text, elapsed) = run_pipe_session(&home, &session_lines(&workload), 180);
    assert_clean_load(&text);
    assert!(
        elapsed < Duration::from_secs(60),
        "interactive source of the nvm.sh-shaped workload took {elapsed:?}; \
         the #2023 quadratic alias-arm shape is back (healthy: seconds)"
    );
}

#[test]
fn interactive_pty_source_of_large_definition_file_is_bounded() {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("no script(1) on this host; skipping");
        return;
    }
    let home = sandbox("pty");
    let workload = write_workload(&home);
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
    let lines = session_lines(&workload);
    let writer = std::thread::spawn(move || {
        for line in &lines {
            if stdin.write_all(format!("{line}\n").as_bytes()).is_err() {
                break;
            }
            let _ = stdin.flush();
            std::thread::sleep(Duration::from_millis(300));
        }
    });

    let mut stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    let started = Instant::now();
    let buf = match rx.recv_timeout(Duration::from_secs(120)) {
        Ok(buf) => buf,
        Err(_) => {
            let _ = child.kill();
            panic!(
                "interactive PTY session did not finish within 120s; \
                 the #2023 busy-spin shape is back"
            );
        }
    };
    let elapsed = started.elapsed();
    let status = child.wait().expect("wait for script(1)");
    let _ = writer.join();
    let _ = reader.join();
    assert!(status.success(), "script(1) PTY session failed: {status:?}");
    let text = String::from_utf8_lossy(&buf).into_owned();
    assert_clean_load(&text);
    assert!(
        elapsed < Duration::from_secs(90),
        "PTY interactive source of the nvm.sh-shaped workload took {elapsed:?}; \
         the #2023 quadratic alias-arm shape is back (healthy: seconds)"
    );
}
