//! Regression pins for rubash#3093 — in an interactive session the
//! first DEBUG-trap firing for a command must already see that
//! command in the history list, and the trap must fire exactly once
//! per simple command.
//!
//! Base behaviour (niubash 1.5.0 / rubash 29bbf524, reporter's trace):
//! the trap fires two to three times per command and the first
//! firing's `history 1` still shows the *previous* command, so a
//! preexec hook that latches state on its first call (VS Code's
//! `__vsc_preexec_all`, whose 633;E payload is `history 1`) keeps the
//! stale value for every command.
//!
//! GNU ownership: an interactive line is recorded on the history
//! list before it is executed (bashhist.c pre_process_line /
//! maybe_add_history in the reader loop), and the DEBUG trap runs
//! once per simple command from execute_command_internal's
//! run_debug_trap call sites (trap.c:1263). A hook sampling
//! `history 1` on the first firing therefore reads the command that
//! is about to run.
//!
//! Expectations are the GNU bash 5.3.20 outputs for the same
//! sessions, driven through the same `script(1)` PTY harness with an
//! empty HOME: H sequence `[3 echo one] [4 echo two] [5 exit]`,
//! firing count 2 at the counting echo, latch payload `echo one`.
//! History numbers are normalized away before comparison — the pin
//! judges ordering and firing count, not numbering cosmetics.
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3093-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Drive one interactive session through a `script(1)` PTY (the
/// issue1016 harness shape): paced line writes, whole transcript
/// collected under a hard timeout so a hung session fails the cell
/// instead of hanging the suite.
fn run_pty_session(home: &Path, lines: &[&str]) -> String {
    if Command::new("script").arg("--version").output().is_err() {
        panic!("script(1) is required for the interactive pins");
    }
    let cmdline = format!("{BIN} -i");
    let mut child = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .env("TERM", "dumb")
        .env("HOME", home)
        .env("HISTFILE", home.join(".hist"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn script(1)");

    let mut stdin = child.stdin.take().expect("child stdin");
    let owned: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
    let writer = std::thread::spawn(move || {
        // Let the shell reach its first prompt before typing.
        std::thread::sleep(Duration::from_secs(2));
        for line in owned {
            if stdin.write_all(line.as_bytes()).is_err() {
                break;
            }
            if stdin.write_all(b"\n").is_err() {
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

    let buf = match rx.recv_timeout(Duration::from_secs(90)) {
        Ok(buf) => buf,
        Err(_) => {
            let _ = child.kill();
            panic!("interactive session did not finish within 90s");
        }
    };
    let _ = writer.join();
    let _ = reader.join();
    let _ = child.wait();
    String::from_utf8_lossy(&buf).replace('\r', "")
}

/// Extract the payloads of `TAG=[<num>  <command>]` samples from a
/// session transcript, normalized to just the command text. Echoed
/// input lines carry the literal `%s` form, never a number, so the
/// digit requirement keeps them out.
fn extract_samples(transcript: &str, tag: &str) -> Vec<String> {
    let needle = format!("{tag}=[");
    let mut out = Vec::new();
    let mut rest = transcript;
    while let Some(pos) = rest.find(&needle) {
        let after = &rest[pos + needle.len()..];
        let Some(end) = after.find(']') else { break };
        let inner = after[..end].trim_start();
        // Split off the leading history number.
        let digits: usize = inner.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            out.push(inner[digits..].trim_start().to_string());
        }
        rest = &after[end + 1..];
    }
    out
}

#[test]
fn first_debug_firing_sees_the_current_command() {
    let home = scratch("hseq");
    let transcript = run_pty_session(
        &home,
        &[
            "f() { printf 'H=[%s]\\n' \"$(history 1)\" >&2; }",
            "trap 'f' DEBUG",
            "echo one",
            "echo two",
            "exit",
        ],
    );
    let samples = extract_samples(&transcript, "H");
    assert_eq!(
        samples,
        vec![
            "echo one".to_string(),
            "echo two".to_string(),
            "exit".to_string()
        ],
        "transcript:\n{transcript}"
    );
}

#[test]
fn debug_trap_fires_once_per_command() {
    let home = scratch("count");
    let transcript = run_pty_session(
        &home,
        &[
            "n=0",
            "c() { n=$((n+1)); }",
            "trap 'c' DEBUG",
            "echo one",
            "echo \"firings=$n\"",
            "exit",
        ],
    );
    // GNU: the counting echo's own firing makes 2 (one per command).
    assert!(
        transcript.contains("firings=2"),
        "transcript:\n{transcript}"
    );
    for wrong in ["firings=3", "firings=4", "firings=5", "firings=6"] {
        assert!(
            !transcript.contains(wrong),
            "unexpected {wrong} in transcript:\n{transcript}"
        );
    }
}

#[test]
fn latching_preexec_hook_reads_the_current_command() {
    let home = scratch("latch");
    let transcript = run_pty_session(
        &home,
        &[
            "latched=",
            "hook() { if [ -z \"$latched\" ]; then latched=$(history 1); fi; }",
            "trap 'hook' DEBUG",
            "echo one",
            "printf 'LATCHED=[%s]\\n' \"$latched\"",
            "exit",
        ],
    );
    let samples = extract_samples(&transcript, "LATCHED");
    assert_eq!(
        samples,
        vec!["echo one".to_string()],
        "transcript:\n{transcript}"
    );
}

/// Script-form control (regression lock): with `set -o history` the
/// non-interactive path already orders the history write before the
/// trap on the base; this cell keeps it that way.
#[test]
fn script_form_history_precedes_the_trap() {
    let root = scratch("script");
    let script = root.join("case.sh");
    std::fs::write(
        &script,
        "set -o history\nhistory -c\nf() { printf 'H=[%s]\\n' \"$(history 1)\" >&2; }\ntrap 'f' DEBUG\necho one\necho two\n",
    )
    .expect("case script");
    let out = Command::new(BIN)
        .arg(&script)
        .current_dir(&root)
        .env("LC_ALL", "C.UTF-8")
        .env("HOME", &root)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let samples = extract_samples(&stderr, "H");
    assert_eq!(
        samples,
        vec!["echo one".to_string(), "echo two".to_string()],
        "stderr:\n{stderr}"
    );
    assert_eq!(out.stdout, "one\ntwo\n");
}
