//! Bridge control-byte regressions (rubash#434).
//!
//! A line delivered through a winpty/ConPTY bridge is an encoding of typed
//! TEXT, not a keystroke stream: a literal `\1` typed by the user used to
//! reach the interactive reader as byte 0x01 and was honored as console
//! Ctrl-A (cursor-home), silently corrupting the edit buffer; the sibling
//! niubash#451 `hang:source:^Ucommand echo ECOSRC` family injected 0x15
//! (Ctrl-U/NAK) the same way. The interactive reader now normalizes each
//! cooked line before the edit dispatch: C0 bytes without cooked-line
//! semantics are dropped, so they can neither act on nor enter the
//! command-line buffer, while the keys that keep line-level semantics
//! (Enter/Tab/Backspace/C-c/C-d) still work (no regression for real keys).

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

fn run_stdin_with_timeout(stdin: &[u8]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-i"])
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
    // A crude hang guard: the injection families this pins used to wedge the
    // reader, so a missing exit must fail the test, not the suite runner.
    for _ in 0..300 {
        match child.try_wait().expect("wait rubash") {
            Some(_) => break,
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// rubash#434: a bridge-delivered 0x01 must not act as console Ctrl-A. The
/// old dispatch moved the cursor to column 0 mid-line, so `echo \x01x`
/// executed as `xecho ` (the `x` re-inserted at the buffer head); the
/// normalized line keeps the whole text and runs `echo x`'s text intact.
#[test]
fn bridge_ctrl_a_byte_does_not_corrupt_the_edit_buffer() {
    let (stdout, _stderr, code) = run_stdin_with_timeout(b"echo A\x01B\nexit\n");
    assert_eq!(stdout, "AB\n", "0x01 corrupted the line: {stdout:?}");
    assert_eq!(code, Some(0));
}

/// niubash#451 family: an injected 0x15 (Ctrl-U/NAK) must not kill the text
/// already in the buffer (`echo A\x15B` used to run as `B`).
#[test]
fn bridge_ctrl_u_byte_does_not_kill_the_buffer() {
    let (stdout, _stderr, code) = run_stdin_with_timeout(b"echo A\x15B\nexit\n");
    assert_eq!(stdout, "AB\n", "0x15 killed the buffer: {stdout:?}");
    assert_eq!(code, Some(0));
}

/// A 0x15-prefixed command line (the exact `hang:source:^Ucommand echo ...`
/// shape) neither hangs the reader nor corrupts the command word.
#[test]
fn bridge_nak_prefixed_line_runs_without_hanging() {
    let (stdout, _stderr, code) = run_stdin_with_timeout(b"\x15echo NAK-OK\nexit\n");
    assert_eq!(stdout, "NAK-OK\n", "0x15-prefixed line: {stdout:?}");
    assert_eq!(code, Some(0));
}

/// Other unbound C0 artifacts (NUL, BEL, DC3, SYN, SUB, FS) are dropped the
/// same way — one normalization layer, not per-byte patches.
#[test]
fn bridge_unbound_c0_bytes_are_dropped() {
    let (stdout, _stderr, code) =
        run_stdin_with_timeout(b"ec\x00ho \x07C\x13\x17\x19\x1a\x1c0\nexit\n");
    assert_eq!(stdout, "C0\n", "unbound C0 bytes leaked: {stdout:?}");
    assert_eq!(code, Some(0));
}

/// Keys with cooked-line semantics survive normalization: backspace (DEL)
/// still edits, so real keypress behavior does not regress.
#[test]
fn bridge_backspace_still_edits_the_buffer() {
    let (stdout, _stderr, code) = run_stdin_with_timeout(b"ec\x7fcho BS-OK\nexit\n");
    assert_eq!(stdout, "BS-OK\n", "DEL backspace regressed: {stdout:?}");
    assert_eq!(code, Some(0));
}

/// C-c still discards the edit line and C-d still ends the session.
#[test]
fn bridge_ctrl_c_and_ctrl_d_keep_their_semantics() {
    let (stdout, _stderr, code) = run_stdin_with_timeout(b"echo DROP\x03\necho KEPT\n\x04");
    assert!(
        !stdout.contains("DROPPED") && stdout.contains("KEPT\n"),
        "C-c/C-d semantics regressed: {stdout:?}"
    );
    assert_eq!(code, Some(0));
}

/// Arrow-key escape sequences sent by a bridge are consumed whole: the ESC
/// introducer survives normalization, so the sequence dispatch (not the
/// buffer) sees it and the `[A` tail never leaks into the command text.
#[test]
fn bridge_arrow_key_tail_never_leaks_into_the_buffer() {
    let (stdout, _stderr, _code) = run_stdin_with_timeout(b"echo HIST-1\n\x1b[A\nexit\n");
    assert_eq!(
        stdout, "HIST-1\n",
        "[A tail leaked into the buffer: {stdout:?}"
    );
}
