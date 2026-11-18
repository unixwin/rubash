//! Issue rubash#3092: `${ cmd; }` (bash 5.3 no-fork command substitution)
//! runs in a subshell when evaluated as part of PS0/PS1 — BASH_SUBSHELL
//! is 1 there and assignments made inside the substitution are lost.
//! The same body on the command line and inside PROMPT_COMMAND is
//! correct (BASH_SUBSHELL 0, assignments survive).
//!
//! GNU contract (bash 5.3 C source):
//! - subst.c:4401 `expand_prompt_string` is the same word expansion as
//!   any other channel (`expand_word_internal`), so a `${ cmd; }` inside
//!   a prompt string reaches subst.c:6917 `function_substitute`, which
//!   runs the body with subst.c:7101 `parse_and_execute ("nofork
//!   comsub")` in the CURRENT shell — no fork, no subshell level.
//! - parse.y:6684-6697: the prompt render never disturbs the reader's
//!   status words ($? / PIPESTATUS / comsub status).
//! - The nofork body still runs with a function-like frame:
//!   `local` scopes to the body and `return` ends only the body
//!   (subst.c:7020-7030, comsub2.tests), while plain assignments
//!   mutate the current environment.
//! - A plain `$(...)` inside the same prompt DOES fork
//!   (BASH_SUBSHELL 1) — the nofork form is the only exception.
//!
//! Rubash divergence (base 763fa0a0): the rubash `-i` driver's prompt
//! render uses the `&self` channel `expand_prompt_string`, whose
//! embedded walker runs a funsub through `function_substitute` in a
//! CLONED executor (`command_substitution_executor`, subshell_depth
//! +1) — BASH_SUBSHELL reads 1 inside and the clone's assignments die
//! with it. The command-word channel and the Session host channel use
//! the `&mut` walker, whose funsub runs in the current shell
//! (`execute_current_shell_body`). The fix unifies the render channel
//! onto the mut walker.
//!
//! Harness shape (niu #769 lesson, same as #3051): `script(1)` only
//! allocates the pty; a parent `/usr/bin/bash -i` holds the session
//! and runs rubash as its foreground job, so rubash's pgrp is not
//! orphaned. Where script(1) is unavailable the PTY cells return
//! without asserting.

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run(exec: &mut Executor, src: &str) {
    let ast = parse(&tokenize(src));
    exec.execute_ast(&ast).ok();
}

/// The PS1 render channel runs a `${ cmd; }` body in the current
/// shell: BASH_SUBSHELL reads 0 inside and the assignment survives.
#[test]
fn ps1_funsub_runs_in_current_shell() {
    let mut exec = Executor::new();
    let rendered =
        exec.expand_prompt_string(r#"${ P1=yes; printf "[PS1sub=%s]" "$BASH_SUBSHELL"; }"#);
    assert_eq!(rendered, "[PS1sub=0]");
    assert_eq!(exec.get_env("P1"), Some("yes"));
}

/// Same for the PS0 render channel.
#[test]
fn ps0_funsub_runs_in_current_shell() {
    let mut exec = Executor::new();
    let rendered =
        exec.expand_prompt_string(r#"${ P0=yes; printf "PS0sub=%s" "$BASH_SUBSHELL"; }"#);
    assert_eq!(rendered, "PS0sub=0");
    assert_eq!(exec.get_env("P0"), Some("yes"));
}

/// A `local` inside a prompt funsub scopes to the body (GNU
/// function-like frame): the value prints, then the name is unset.
#[test]
fn prompt_funsub_local_scopes_to_body() {
    let mut exec = Executor::new();
    let rendered = exec.expand_prompt_string(r#"${ local lv=scoped; printf "%s" "$lv"; }"#);
    assert_eq!(rendered, "scoped");
    assert_eq!(exec.get_env("lv"), None);
}

/// A `return` inside a prompt funsub ends only the body: the text
/// after it never runs, and the render result is empty.
#[test]
fn prompt_funsub_return_ends_body_only() {
    let mut exec = Executor::new();
    let rendered = exec.expand_prompt_string(r#"${ return 3; echo UNREACHED; }"#);
    assert_eq!(rendered, "");
}

/// A nested funsub inside a prompt funsub expands in order.
#[test]
fn prompt_funsub_nested() {
    let mut exec = Executor::new();
    let rendered = exec.expand_prompt_string(r#"${ echo "A${ echo B; }C"; }"#);
    assert_eq!(rendered, "ABC");
}

/// Guard: a plain `$(...)` inside the same prompt still forks —
/// BASH_SUBSHELL reads 1. The nofork form is the only exception.
#[test]
fn prompt_dollar_paren_still_forks() {
    let mut exec = Executor::new();
    let rendered = exec.expand_prompt_string(r#"$(printf "%s" "$BASH_SUBSHELL")"#);
    assert_eq!(rendered, "1");
}

/// Guard: the Session host channel (`expand_prompt_string_mut`,
/// what niu calls) already runs funsub in the current shell — the
/// render channel must agree with it byte-for-byte.
#[test]
fn mut_channel_funsub_parity() {
    let mut exec = Executor::new();
    let rendered =
        exec.expand_prompt_string_mut(r#"${ P1=yes; printf "[PS1sub=%s]" "$BASH_SUBSHELL"; }"#);
    assert_eq!(rendered, "[PS1sub=0]");
    assert_eq!(exec.get_env("P1"), Some("yes"));
}

/// Guard: the command-line channel is the known-good reference —
/// a funsub there runs in the current shell and keeps assignments.
#[test]
fn command_line_funsub_control() {
    let mut exec = Executor::new();
    run(&mut exec, r#"out=${ X=done; printf inner; }"#);
    assert_eq!(exec.get_env("X"), Some("done"));
    assert_eq!(exec.get_env("out"), Some("inner"));
}

/// Guard: the prompt render never disturbs the reader's `$?`
/// (parse.y:6684-6697) — a failing funsub body leaves the
/// pre-render status in place.
#[test]
fn prompt_render_preserves_exit_status() {
    let mut exec = Executor::new();
    run(&mut exec, "false");
    assert_eq!(exec.last_exit_code(), 1);
    let rendered = exec.expand_prompt_string(r#"${ false; printf R; }"#);
    assert_eq!(rendered, "R");
    assert_eq!(exec.last_exit_code(), 1);
}

// ---------------------------------------------------------------------------
// PTY chain cells: the full issue repro through a real interactive
// session (script(1) + parent bash -i + rubash -i).
// ---------------------------------------------------------------------------

#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::process::{Child, Command, Stdio};
#[cfg(unix)]
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
struct PtySession {
    child: Child,
    stdin: std::process::ChildStdin,
    transcript: Arc<Mutex<String>>,
}

#[cfg(unix)]
impl PtySession {
    /// Spawn `script(1)` running `command` on the pty.
    fn spawn(command: &str) -> Option<PtySession> {
        if Command::new("script").arg("--version").output().is_err() {
            return None;
        }
        let home = std::env::temp_dir().join(format!("rubash3092-home-{}", std::process::id()));
        let _ = fs::create_dir_all(&home);
        let mut child = Command::new("script")
            .args(["-qec", command, "/dev/null"])
            .env("TERM", "xterm")
            .env("HOME", &home)
            .env("PS1", "PARENT$ ")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn script(1)");
        let stdin = child.stdin.take().expect("script stdin");
        let mut stdout = child.stdout.take().expect("script stdout");
        let transcript = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&transcript);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&chunk[..n]).into_owned();
                        sink.lock().expect("transcript lock").push_str(&text);
                    }
                }
            }
        });
        Some(PtySession {
            child,
            stdin,
            transcript,
        })
    }

    fn send(&mut self, bytes: &str) {
        let _ = self.stdin.write_all(bytes.as_bytes());
        let _ = self.stdin.flush();
    }

    fn text(&self) -> String {
        self.transcript.lock().expect("transcript lock").clone()
    }

    /// Wait until `needle` appears in the transcript at or after byte
    /// offset `from`. Returns the offset just past the match.
    fn wait_for(&self, needle: &str, from: usize, timeout: Duration) -> usize {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let text = self.text();
                if let Some(pos) = text[from..].find(needle) {
                    return from + pos + needle.len();
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle:?}; transcript so far: {:?}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn finish(mut self) -> std::process::ExitStatus {
        self.send("exit 0\n");
        std::thread::sleep(Duration::from_millis(300));
        self.send("exit 0\n");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self.child.try_wait().expect("poll script(1)") {
                Some(status) => return status,
                None => {
                    assert!(
                        Instant::now() < deadline,
                        "session did not exit; transcript: {:?}",
                        self.text()
                    );
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }
}

#[cfg(unix)]
impl Drop for PtySession {
    fn drop(&mut self) {
        // Never leak a stopped session behind a failed assert.
        let _ = self.child.kill();
    }
}

#[cfg(unix)]
const LONG: Duration = Duration::from_secs(20);

/// Start the parent + rubash child and install the issue repro's
/// PS0/PS1/PROMPT_COMMAND. Returns the session plus the transcript
/// offset from which the probe output must be judged.
#[cfg(unix)]
fn prompt_session() -> Option<(PtySession, usize)> {
    let mut session = PtySession::spawn("/usr/bin/bash --norc -i")?;
    std::thread::sleep(Duration::from_millis(800));
    session.send(&format!("{} -i\n", env!("CARGO_BIN_EXE_rubash")));
    std::thread::sleep(Duration::from_millis(1000));
    session.send(r#"PS0='${ P0=yes; printf "PS0sub=%s\n" "$BASH_SUBSHELL"; }'"#);
    session.send("\n");
    std::thread::sleep(Duration::from_millis(500));
    session.send(r#"PS1='${ P1=yes; printf "[PS1sub=%s]" "$BASH_SUBSHELL"; }$ '"#);
    session.send("\n");
    std::thread::sleep(Duration::from_millis(500));
    session.send(r#"PROMPT_COMMAND='${ PC=yes; }'"#);
    session.send("\n");
    std::thread::sleep(Duration::from_millis(500));
    let probe_from = session.text().len();
    Some((session, probe_from))
}

/// The issue repro end to end: PS0/PS1 funsub bodies run in the
/// current shell (BASH_SUBSHELL 0) and their assignments survive.
#[cfg(unix)]
#[test]
fn pty_ps0_ps1_funsub_runs_in_current_shell() {
    let Some((mut session, probe_from)) = prompt_session() else {
        eprintln!("no script(1) on this host; skipping");
        return;
    };
    session.send(r#"echo "P0=[${P0:-unset}] P1=[${P1:-unset}] PC=[${PC:-unset}]""#);
    session.send("\n");
    let _ = session.wait_for("P0=[yes] P1=[yes] PC=[yes]", probe_from, LONG);
    let transcript = session.text();
    let tail = &transcript[probe_from..];
    assert!(
        tail.contains("PS0sub=0"),
        "PS0 funsub must run in the current shell (BASH_SUBSHELL 0); transcript: {transcript:?}"
    );
    assert!(
        tail.contains("[PS1sub=0]"),
        "PS1 funsub must run in the current shell (BASH_SUBSHELL 0); transcript: {transcript:?}"
    );
    assert!(
        !tail.contains("PS0sub=1") && !tail.contains("[PS1sub=1]"),
        "no prompt funsub may report a subshell; transcript: {transcript:?}"
    );
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {transcript:?}");
}

/// Guard: a plain `$(...)` in PS1 still forks inside a live
/// interactive session.
#[cfg(unix)]
#[test]
fn pty_prompt_dollar_paren_still_forks() {
    let mut session = match PtySession::spawn("/usr/bin/bash --norc -i") {
        Some(s) => s,
        None => {
            eprintln!("no script(1) on this host; skipping");
            return;
        }
    };
    std::thread::sleep(Duration::from_millis(800));
    session.send(&format!("{} -i\n", env!("CARGO_BIN_EXE_rubash")));
    std::thread::sleep(Duration::from_millis(1000));
    session.send(r#"PS1='$(printf "dollarsub=%s" "$BASH_SUBSHELL")$ '"#);
    session.send("\n");
    std::thread::sleep(Duration::from_millis(500));
    let probe_from = session.text().len();
    session.send("echo hi\n");
    let _ = session.wait_for("dollarsub=1", probe_from, LONG);
    let transcript = session.text();
    assert!(
        transcript[probe_from..].contains("dollarsub=1"),
        "plain $(...) in PS1 must still fork; transcript: {transcript:?}"
    );
    let status = session.finish();
    assert_eq!(status.code(), Some(0), "transcript: {transcript:?}");
}
