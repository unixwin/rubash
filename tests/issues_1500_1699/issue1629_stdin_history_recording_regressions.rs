//! rubash#1629 — the non-interactive stdin driver recorded input lines
//! into the history list. GNU: maybe_add_history is gated on
//! remember_on_history (the `history` set option), which defaults OFF
//! for a non-interactive shell, so lines read from stdin are NOT
//! recorded; only explicit `history -s` entries appear.
//!
//! Expectations verified against GNU bash 5.3.20 (piped stdin).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(input: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let mut stdin = child.stdin.take().expect("stdin pipe");
    stdin.write_all(input.as_bytes()).expect("write stdin");
    drop(stdin);
    let output = child.wait_with_output().expect("wait rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn stdin_lines_are_not_recorded() {
    // The issue's seed: only the -s entry may appear.
    let stdout = run_stdin("history -s \"echo via-stdin\"\nhistory\n");
    assert_eq!(stdout, "    1  echo via-stdin\n");
}

#[test]
fn stdin_lines_after_first_builtin_are_not_recorded() {
    // Once the first history builtin creates the session list, later
    // input lines still must not be recorded (GNU records nothing in
    // default stdin mode).
    let stdout = run_stdin("history -s marker\necho plain\nhistory\n");
    assert_eq!(stdout, "plain\n    1  marker\n");
}

#[test]
fn stdin_listing_is_empty_without_explicit_entries() {
    let stdout = run_stdin("echo a\nhistory\n");
    assert_eq!(stdout, "a\n");
}
