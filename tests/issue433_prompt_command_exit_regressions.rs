//! Issue rubash#433 (lane wt99/i431): `PROMPT_COMMAND='exit'` killed GNU
//! bash at the prompt, but rubash's interactive session survived — the
//! exit jump raised inside the PROMPT_COMMAND text was consumed by the
//! grouped runner and never re-raised, so the shell kept prompting and
//! read the rest of stdin.
//!
//! GNU spec (bash.git @b4608166, Bash-5.3 patch 15):
//! - eval.c:318-330 `execute_variable_command` runs the PC text through
//!   `parse_and_execute(..., SEVAL_NONINT|SEVAL_NOHIST|SEVAL_NOOPTIMIZE|
//!   SEVAL_NOTIFY)` (y.tab.c:5374) in the CURRENT shell.
//! - builtins/evalstring.c:396-403: parse_and_execute catches EXITPROG at
//!   its own `setjmp (top_level)` and sets `should_jump_to_top_level = 1`;
//!   :618-619 re-raises `jump_to_top_level (code)` after the `out:` cleanup
//!   — the jump unwinds execute_variable_command / execute_array_command
//!   (eval.c:291-301, elements after an `exit` element never run) into
//!   reader_loop, which ends the shell with the jump's status.
//! - parse.y:3013/3021: execute_variable_command brackets the run with
//!   save_parser_state/restore_parser_state; the snapshot carries
//!   last_command_exit_value (parse.y:7221) and PIPESTATUS (:7223) and is
//!   written back at :7313/:7315 on every NORMAL return — a failing PC
//!   leaves `$?` at its pre-PC value, and the restore line is skipped when
//!   the exit jump longjmps past it (PC='exit 5' -> rc 5).
//! - evalstring.c:284-285/:612: SEVAL_NONINT zeroes `interactive` for the
//!   duration, so exit.def:59-62's interactive "exit" echo is suppressed
//!   while the PC text runs (GNU's dying session prints no "exit" line).
//!
//! Verified byte-for-byte against WSL GNU Bash 5.3.0 piped-`-i` probes
//! (target/issue-suites/results/wt99-i431/pc-*.sh): PC='exit' -> rc 0, no
//! stdout, exactly one prompt, no "exit" echo; PC array with a mid-array
//! `exit` -> only the earlier elements' output; PC='exit 5' -> rc 5;
//! PC='set -e; false' -> rc 1; PC='false' -> the session survives with the
//! pre-PC `$?`.

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct Session {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Drives one piped-`-i` interactive session: `rubash -i --norc` with the
/// given lines on stdin (stdin closed after them, the GNU probe shape).
fn run_interactive(lines: &str) -> Session {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .arg("--norc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(lines.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    Session {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// PC='exit' at the first prompt: the shell ends BEFORE reading or running
/// the next line — stdout empty, rc 0, one prompt, no interactive "exit"
/// echo (SEVAL_NONINT, evalstring.c:284-285).
#[test]
fn pc_exit_string_ends_session_before_next_line() {
    let session = run_interactive("PROMPT_COMMAND='exit'\necho AFTER-PC\n");
    assert_eq!(session.code, Some(0), "stderr: {}", session.stderr);
    assert!(
        session.stdout.is_empty(),
        "the line after the PC must never run: {:?}",
        session.stdout
    );
    assert_eq!(
        session.stderr.matches("bash-5.3#").count(),
        1,
        "exactly one prompt is rendered: {:?}",
        session.stderr
    );
    assert!(
        !session.stderr.lines().any(|l| l.trim() == "exit"),
        "SEVAL_NONINT suppresses the interactive exit echo: {:?}",
        session.stderr
    );
}

/// eval.c:291-301: the longjmp unwinds execute_array_command — elements
/// after the `exit` element never run.
#[test]
fn pc_exit_element_stops_remaining_array_elements() {
    let session = run_interactive(
        "PROMPT_COMMAND=([0]='echo PC-A' [1]='exit' [2]='echo PC-C')\necho AFTER-PC\n",
    );
    assert_eq!(session.code, Some(0), "stderr: {}", session.stderr);
    assert_eq!(session.stdout, "PC-A\n", "stderr: {}", session.stderr);
}

/// The re-raised jump carries the exit status (parse.y:3021's restore is
/// skipped, like GNU's longjmp past it).
#[test]
fn pc_exit_n_uses_the_given_status() {
    let session = run_interactive("echo BEFORE\nPROMPT_COMMAND='exit 5'\necho AFTER-PC\n");
    assert_eq!(session.code, Some(5), "stderr: {}", session.stderr);
    assert_eq!(session.stdout, "BEFORE\n", "stderr: {}", session.stderr);
}

/// evalstring.c:387-393: ERREXIT joins EXITPROG in the re-raise.
#[test]
fn pc_errexit_break_ends_session() {
    let session = run_interactive("PROMPT_COMMAND='set -e; false'\necho AFTER-PC\n");
    assert_eq!(session.code, Some(1), "stderr: {}", session.stderr);
    assert!(
        session.stdout.is_empty(),
        "the line after the PC must never run: {:?}",
        session.stdout
    );
}

/// The session must SURVIVE a PC that merely fails (no errexit), and
/// parse.y:7313/:7315 restore `$?` to the pre-PC value.
#[test]
fn pc_failing_status_survives_with_pre_pc_dollar_question() {
    let session = run_interactive("PROMPT_COMMAND='false'\necho AFTER-FALSE\necho ST=$?\n");
    assert_eq!(session.code, Some(0), "stderr: {}", session.stderr);
    assert!(
        session.stdout.contains("AFTER-FALSE"),
        "the shell must prompt again after a plain failing PC: {:?}",
        session.stdout
    );
    assert!(
        session.stdout.contains("ST=0"),
        "$? must keep the pre-PC value: {:?}",
        session.stdout
    );
    assert!(
        session.stderr.matches("bash-5.3#").count() >= 3,
        "the reader must render the follow-up prompts: {:?}",
        session.stderr
    );
}

/// A PC parse error is a DISCARD, not an unwind (evalstring.c:585-601
/// records the failure and breaks without a jump): the session survives.
/// The residual final-rc accounting of a session ENDING on a parse abort
/// is a separate pre-existing reader matter (both shells exit 2 on a
/// trailing READER-level parse error; a trailing PC parse error leaves GNU
/// rc 0 because last_command_exit_value is not written there).
#[test]
fn pc_parse_error_discards_and_survives() {
    let session = run_interactive("PROMPT_COMMAND='echo )'\necho AFTER-PE\n");
    assert!(
        session.stdout.contains("AFTER-PE"),
        "the session must survive a PC parse error: {:?} {:?}",
        session.stdout,
        session.stderr
    );
}
