//! rubash#2214 — the async start notice `[<job>] <pid>` belongs to
//! the interactive context that STARTS the job, not to the session.
//!
//! GNU's print site is DESCRIBE_PID (execute_cmd.c:609 —
//! `if (interactive) describe_pid (pid)`, jobs.c:1947), and every
//! subshell environment clears `interactive` (execute_cmd.c:1637
//! execute_in_subshell: "Subshells are neither login nor
//! interactive"). Verified against GNU bash 5.3.20 under a real PTY:
//!
//!   `sleep 30 &` at the prompt            -> `[1] <pid>` printed
//!   `f() { sleep 30 & }; f`               -> printed (function body
//!                                            runs in the main context)
//!   `( sleep 30 & )`                      -> NOT printed
//!   `x=$( sleep 30 & echo hi )`           -> NOT printed
//!   `( sleep 30 & wait ) | cat`           -> NOT printed
//!   `coproc sleep 30`                     -> printed
//!     (execute_cmd.c:2580 DESCRIBE_PID (coproc_pid))
//!
//! rubash gated the notice on the session's `__RUBASH_INTERACTIVE`
//! marker alone; the fork copies executing subshell environments
//! carry that marker in their cloned state, so children started
//! inside `( )` / `$( )` / pipeline stages printed stray notices —
//! observed in the field as interactive `nvm ls` output interleaved
//! with `[1] <pid>` tokens from nvm's `( ... & ... ) | sort` alias
//! printers. The fix gates the notify on
//! `in_subshell_environment()` as well, and routes the coproc spawn
//! (which never notified) through the same chokepoint.
//!
//! These pins drive `rubash -i` over pipes (the notice gate is the
//! session's interactive marker, set for `-i` with or without a
//! tty); each shape runs in its own session so the notice count is
//! attributable. A start notice is the bare token `[<n>] <pid>`;
//! completion notices (`[1]+ Done ...`) and `jobs` listings carry a
//! `+` after the bracket and never match the counter.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2214-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create sandbox dir");
    dir
}

/// Run one interactive pipe session; return (stdout, stderr).
fn run_session(home: &PathBuf, lines: &[&str]) -> (String, String) {
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
    let mut script = lines.join("\n");
    script.push_str("\nexit\n");
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(script.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    });
    let (tx, rx) = mpsc::channel();
    let mut out = child.stdout.take().expect("child stdout");
    let t_out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let (tx2, rx2) = mpsc::channel();
    let mut err = child.stderr.take().expect("child stderr");
    let t_err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        let _ = tx2.send(buf);
    });
    let deadline = Instant::now() + Duration::from_secs(60);
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
    (
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    )
}

/// Count async START notices (`[<n>] <pid>`) in the session's
/// stderr. Completion notices and jobs listings have a `+` right
/// after the `]`, so they never match.
fn start_notice_count(stderr: &str) -> usize {
    let bytes = stderr.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            // [<digits>] <digits>
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < bytes.len() && bytes[j] == b']' {
                let k = j + 1;
                if k < bytes.len() && bytes[k] == b' ' {
                    let mut m = k + 1;
                    while m < bytes.len() && bytes[m].is_ascii_digit() {
                        m += 1;
                    }
                    if m > k + 1 {
                        count += 1;
                        i = m;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    count
}

fn expect_notices(tag: &str, lines: &[&str], expected: usize, stdout_needles: &[&str]) {
    let home = sandbox(tag);
    let (stdout, stderr) = run_session(&home, lines);
    for needle in stdout_needles {
        assert!(
            stdout.contains(needle),
            "{tag}: session output missing {needle:?}; stdout: {stdout:?} stderr: {stderr:?}"
        );
    }
    assert_eq!(
        start_notice_count(&stderr),
        expected,
        "{tag}: wrong async start-notice count; stderr: {stderr:?} stdout: {stdout:?}"
    );
    let _ = fs::remove_dir_all(&home);
}

/// Background children of a `( )` subshell: no notice (GNU).
#[test]
fn no_start_notice_inside_paren_subshell() {
    expect_notices(
        "subshell",
        &["( sleep 1 & wait ); echo DONE-SUB"],
        0,
        &["DONE-SUB"],
    );
}

/// Background children of a command substitution: no notice (GNU).
#[test]
fn no_start_notice_inside_command_substitution() {
    expect_notices(
        "comsub",
        &["x=$( sleep 1 & echo hi ); echo \"X=$x\""],
        0,
        &["X=hi"],
    );
}

/// Background children of a pipeline-stage subshell: no notice (GNU).
#[test]
fn no_start_notice_inside_pipeline_stage() {
    expect_notices(
        "pipestage",
        &["( sleep 1 & wait ) | cat; echo DONE-PIPE"],
        0,
        &["DONE-PIPE"],
    );
}

/// A function running as a pipeline stage is a subshell environment:
/// its own `&` children print no notice (GNU).
#[test]
fn no_start_notice_inside_function_in_pipeline() {
    expect_notices(
        "funcpipe",
        &["f() { sleep 1 & wait; }; f | cat; echo DONE-FP"],
        0,
        &["DONE-FP"],
    );
}

/// The reported field shape (nvm's alias printers): backgrounded
/// workers inside `( ... ) | sort` inside a function — no notices,
/// and the workers' output still arrives (GNU).
#[test]
fn no_start_notice_in_nvm_shaped_alias_printers() {
    expect_notices(
        "nvmshape",
        &["f() { ( for i in 1 2 3; do echo \"item$i\" & done; wait ) | sort; }; f; echo DONE-NVM"],
        0,
        &["item1", "item2", "item3", "DONE-NVM"],
    );
}

/// Control: a top-level background job DOES print exactly one
/// notice — the gate must not over-suppress (GNU).
#[test]
fn start_notice_at_top_level() {
    expect_notices(
        "toplevel",
        &["sleep 1 & echo DONE-TOP; wait"],
        1,
        &["DONE-TOP"],
    );
}

/// Control: a function body running in the main interactive context
/// prints for its own `&` children (GNU).
#[test]
fn start_notice_inside_function_in_main_context() {
    expect_notices(
        "funcmain",
        &["f() { sleep 1 & }; f; echo DONE-FN; wait"],
        1,
        &["DONE-FN"],
    );
}

/// An async brace group gets ONE notice (for the group job itself,
/// from the parent context); the group's own `&` children run in a
/// subshell environment and print none (GNU).
#[test]
fn one_start_notice_for_async_group_none_inside() {
    expect_notices(
        "asyncgroup",
        &["{ sleep 1 & wait; } & echo DONE-GRP; wait"],
        1,
        &["DONE-GRP"],
    );
}

/// A coproc is an async child of the calling context: exactly one
/// start notice (GNU, execute_cmd.c:2580).
#[test]
fn start_notice_for_coproc() {
    expect_notices(
        "coproc",
        &["coproc sleep 1; echo DONE-COP; wait"],
        1,
        &["DONE-COP"],
    );
}
