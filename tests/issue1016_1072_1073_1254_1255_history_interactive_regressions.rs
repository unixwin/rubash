//! History / interactive regression pins (backfill wave bf-c).
//!
//! rubash#1016 — the interactive REPL stopped consuming input after the
//! first command when the terminal did not answer its status queries:
//! later lines were silently dropped. Pinned here with the repository's
//! pipe-driven `-i` harness form (driven input, no terminal answers):
//! every line must execute, under both a full xterm TERM and dumb TERM.
//! rubash#1072 — `shopt -s histverify`: an interactively recalled line
//! must be HELD for verification, never executed and never echoed as a
//! recall in non-interactive mode; the `!!` line is discarded at EOF.
//! rubash#1073 — `set -o emacs` / `set -o vi` are mutually exclusive in
//! GNU: turning one on turns the other off (all four transitions).
//! rubash#1254 — interactive-only stray `exit` line printed when a
//! subshell / command substitution containing `exit` ran. The leak is
//! in the interactive display path; pinned in pipe-driven `-i` form:
//! no bare `exit` line may appear in either stream.
//! rubash#1255 — `read -p PROMPT` never displayed the prompt on a tty
//! (GNU shows it). TTY-only: pinned through a `script(1)` PTY (unix);
//! off-tty both shells suppress the prompt by design.
//!
//! Expectations hand-measured against GNU bash 5.3.20 in this
//! environment (script files, `-c` probes, and GNU `-i` references).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

mod common;
use common::wait_bounded;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-bfc-hist-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The bounded wait itself is the shared helper in tests/common
/// (rubash#2197): this binary's private copy carried the pre-#2056
/// orphan-pipe hang shape and was deleted in favor of it.

/// Spawn `rubash -i` with piped stdio (the repository's interactive
/// harness form), feed `input`, close stdin, collect both streams.
fn run_interactive(input: &str, term: &str) -> (String, String) {
    let dir = scratch("home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .current_dir(&dir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &dir)
        .env("TERM", term)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(input.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    let outcome = wait_bounded(&mut child, Duration::from_secs(30));
    assert!(
        !outcome.timed_out,
        "child {} did not exit within the bound; partial stderr: {}",
        outcome.pid,
        String::from_utf8_lossy(&outcome.stderr)
    );
    let (out, err) = (outcome.stdout, outcome.stderr);
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("script");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn run_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Parse the `emacs` / `vi` rows out of a full `set -o` listing.
fn emacs_vi_states(set_o_output: &str) -> (String, String) {
    let mut emacs = String::new();
    let mut vi = String::new();
    for line in set_o_output.lines() {
        let mut fields = line.split_whitespace();
        match (fields.next(), fields.next()) {
            (Some("emacs"), Some(state)) => emacs = state.to_string(),
            (Some("vi"), Some(state)) => vi = state.to_string(),
            _ => {}
        }
    }
    (emacs, vi)
}

#[test]
fn issue_1016_repl_consumes_every_driven_line() {
    for term in ["xterm-256color", "dumb"] {
        let (stdout, stderr) = run_interactive("echo ONE\necho TWO\n", term);
        assert!(stdout.contains("ONE"), "TERM={term} stdout: {stdout}");
        assert!(
            stdout.contains("TWO"),
            "TERM={term}: second line dropped; stdout: {stdout} stderr: {stderr}"
        );
    }
}

#[test]
fn issue_1072_histverify_holds_recalled_line_without_executing() {
    let (stdout, stderr, code) = run_script(
        "shopt -s histverify\nset -o history\nset -H\necho FIRST\necho !!\necho \"rc=$?\"\n",
    );
    // GNU: only FIRST runs; the `!!` line is held (discarded at EOF —
    // non-interactive shells cannot verify), no recall echo anywhere.
    assert_eq!(stdout, "FIRST\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "no recall echo; stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1073_emacs_and_vi_options_are_mutually_exclusive() {
    // emacs then vi: vi wins.
    let (out, _e, _c) = run_c("set -o emacs; set -o vi; set -o");
    assert_eq!(emacs_vi_states(&out), ("off".into(), "on".into()), "{out}");
    // vi then emacs: emacs wins.
    let (out, _e, _c) = run_c("set -o vi; set -o emacs; set -o");
    assert_eq!(emacs_vi_states(&out), ("on".into(), "off".into()), "{out}");
    // Turning the current one off leaves both off.
    let (out, _e, _c) = run_c("set -o emacs; set -o vi; set -o emacs; set +o emacs; set -o");
    assert_eq!(emacs_vi_states(&out), ("off".into(), "off".into()), "{out}");
    // Turning off the inactive one changes nothing.
    let (out, _e, _c) = run_c("set -o emacs; set +o vi; set -o");
    assert_eq!(emacs_vi_states(&out), ("on".into(), "off".into()), "{out}");
}

#[test]
fn issue_1254_no_stray_exit_line_for_subshell_or_comsub_exit() {
    for input in [
        "(exit 42)\necho after-sub\n",
        "echo x$(exit 42)y\necho after-com\n",
    ] {
        let (stdout, stderr) = run_interactive(input, "xterm-256color");
        let marker = if input.contains("after-sub") {
            "after-sub"
        } else {
            "after-com"
        };
        assert!(
            stdout.contains(marker),
            "session continued; stdout: {stdout} stderr: {stderr}"
        );
        for stream in [&stdout, &stderr] {
            assert!(
                !stream
                    .lines()
                    .any(|l| l.trim().trim_end_matches('\r') == "exit"),
                "stray bare `exit` line in stream: {stream}"
            );
        }
    }
}

/// rubash#1255: under a real PTY, `read -p` must display its prompt.
/// Driven through util-linux `script(1)`: the typed reply is echoed by
/// the tty line discipline either way, so the prompt string appearing
/// in the typescript can only come from the shell itself.
#[cfg(unix)]
#[test]
fn issue_1255_read_p_prompt_is_displayed_on_a_tty() {
    let dir = scratch("pty");
    let mut child = Command::new("script")
        .args([
            "-qec",
            &format!("{} -i", env!("CARGO_BIN_EXE_rubash")),
            "/dev/null",
        ])
        .current_dir(&dir)
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn script(1) PTY harness");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(b"read -p \"PROMPTXYZ>>> \" a; echo \"a=[$a]\"\nhelloinput\nexit\n")
        .expect("write stdin");
    drop(child.stdin.take());
    let outcome = wait_bounded(&mut child, Duration::from_secs(30));
    assert!(
        !outcome.timed_out,
        "child {} did not exit within the bound; partial stderr: {}",
        outcome.pid,
        String::from_utf8_lossy(&outcome.stderr)
    );
    let typescript = String::from_utf8_lossy(&outcome.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        typescript.contains("a=[helloinput]"),
        "read completed under PTY; typescript: {typescript}"
    );
    assert!(
        typescript.contains("PROMPTXYZ>>>"),
        "prompt displayed on tty; typescript: {typescript}"
    );
}
