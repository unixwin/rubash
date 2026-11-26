//! Real-readline REPL regressions (rubash#419, the readline leg).
//!
//! The console case already routed through the interactive reader
//! (`run_interactive_stdin`, PS1/PS2 on stderr) — what was missing was
//! readline-level line editing on a real console: the edit dispatcher is
//! now factored into `apply_edit_char` and shared by the cooked byte
//! stream and the new raw-console key reader
//! ([`rubash::console_readline`]). These tests pin the dispatcher
//! semantics through the piped-`-i` driver (the byte-stream path keeps the
//! #434 bridge normalization: only backspace/DEL, C-c, C-d, C-v, tab and
//! the ESC arrow introducer survive a cooked line); the raw-console leg
//! itself needs a real console and is covered by
//! `scripts/smoke-console-readline-pty.py` (pywinpty + pyte).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(args: &[&str], stdin: &[u8]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .env_remove("PS1")
        .env_remove("PS2")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Backspace rubs out the preceding character, insert-after-rubout lands
/// in its place: `echo AB` + BS + `C` runs `echo AC`.
#[test]
fn readline_backspace_edits_the_line() {
    let (stdout, stderr, rc) = run_stdin(&["-i"], b"echo AB\x08C\n");
    assert_eq!(stdout, "AC\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// Rubout is character-based, not byte-based: a multibyte character is
/// removed whole, never leaving a stray UTF-8 continuation byte.
#[test]
fn readline_backspace_rubs_out_whole_multibyte_chars() {
    let (stdout, stderr, rc) = run_stdin(&["-i"], "echo é\x7fX\n".as_bytes());
    assert_eq!(stdout, "X\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

/// Arrow-up history recall is a RAW CONSOLE binding only (the new
/// `RawConsole` key reader feeds the dispatcher real key events). A piped
/// line is typed TEXT — rubash#434 pinned that bridge arrow sequences must
/// neither leak a `[A` tail nor recall history on the cooked path. The raw
/// leg is exercised by `scripts/smoke-console-readline-pty.py`.
#[test]
fn piped_arrow_sequence_keeps_bridge_semantics() {
    let (stdout, _stderr, _rc) = run_stdin(&["-i"], b"echo HIST-ENTRY\n\x1b[A\n");
    assert_eq!(stdout, "HIST-ENTRY\n", "bridge arrow semantics changed");
}

/// C-c discards the edit line without ending the session (readline
/// line-discard on SIGINT); the next command still runs.
#[test]
fn readline_ctrl_c_discards_line_and_survives() {
    let (stdout, stderr, rc) = run_stdin(&["-i"], b"echo PARTIAL\x03\necho SURVIVED\n");
    assert_eq!(stdout, "SURVIVED\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
    assert!(!stdout.contains("PARTIAL"));
}

/// C-d on an empty buffer ends the interactive session — the rest of the
/// input is never read (EOF at the primary prompt).
#[test]
fn readline_ctrl_d_on_empty_buffer_ends_session() {
    let (stdout, stderr, rc) = run_stdin(&["-i"], b"echo FIRST\n\x04echo NEVER\n");
    assert_eq!(stdout, "FIRST\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
    assert!(!stdout.contains("NEVER"));
}

/// EOF (closed stdin) mid-buffer accepts the pending line first, then
/// exits (readline's EOF-on-nonempty-buffer rule).
#[test]
fn readline_eof_accepts_pending_line() {
    let (stdout, _stderr, rc) = run_stdin(&["-i"], b"echo FINAL");
    assert_eq!(stdout, "FINAL\n");
    assert_eq!(rc, Some(0));
}

/// The edit stream composes across lines: backspace edits line 1, DEL
/// (0x7f) edits line 2, both accepted lines run.
#[test]
fn readline_editing_composes_across_lines() {
    // line 1: `echo AB` + BS + `2` -> runs `echo A2`
    // line 2: `echo A2` + DEL + `3` -> runs `echo A3`
    let (stdout, stderr, rc) = run_stdin(&["-i"], b"echo AB\x082\necho A2\x7f3\n");
    assert_eq!(stdout, "A2\nA3\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}
