//! Issue rubash#1664 regressions.
//!
//! Backgrounding any job while a function containing a pipe inside
//! `$(...)` was defined corrupted it: rubash serializes all functions
//! into a single-line `-c` payload for the background subshell
//! (`background_function_prelude`), and that serializer had two bugs:
//!
//! 1. The canonical comsub printer (`print_comsub.rs`, GNU print_cmd.c
//!    port) joined pipeline stages with a bare space, dropping `|`:
//!    `$(echo hi | cat)` re-parsed as `$(echo hi cat)` (silent wrong
//!    output `hi cat` instead of `hi`).
//! 2. The background source-text renderer (`bash_command_text` in
//!    command_text.rs) printed assignments from their cooked value,
//!    dropping the source quotes: `x="$(echo a) ($(echo b | cat))"`
//!    became `x=$(echo a) ($(echo b cat))`, a syntax error on re-parse.
//!    GNU print_cmd.c print_simple_command prints each assignment word's
//!    text as lexed (rubash#202, ast_print.rs); the background
//!    serializer now prefers the parser-recorded raw RHS the same way.
//!
//! The multi-line `declare -f` serializer was already correct for both
//! shapes; only the background re-serialization path was wrong.
//! Expected outputs are byte-verified against GNU bash 5.3.20
//! (system bash, script-file probes).

use std::process::Command;

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Silent variant: a pipe inside `$(...)` in a function assignment must
/// survive background re-serialization. Before the fix this printed
/// `hi cat` (the pipe was dropped and `cat` became an argument).
#[test]
fn background_function_comsub_pipeline_keeps_pipe() {
    let (stdout, stderr, code) = rubash_c(r#"f() { x=$(echo hi | cat); echo "$x"; }; f & wait"#);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\n", "stdout: {stdout}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

/// Loud variant: a quoted assignment whose value mixes a comsub and a
/// parenthesized comsub-with-pipe must re-parse. Before the fix the
/// payload lost both the quotes and the pipe, dying with a syntax
/// error near `(`.
#[test]
fn background_function_quoted_comsub_with_pipe_reparses() {
    let (stdout, stderr, code) =
        rubash_c(r#"g() { x="$(echo a) ($(echo b | cat))"; echo "$x"; }; g & wait"#);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a (b)\n", "stdout: {stdout}");
    assert!(
        !stderr.contains("syntax error"),
        "unexpected syntax error: {stderr}"
    );
}

/// Multi-stage pipe chain inside `$(...)`, plus a plain quoted
/// assignment as a control (quotes alone must also survive).
#[test]
fn background_function_comsub_pipe_chain_and_quotes() {
    let (stdout, stderr, code) = rubash_c(
        r#"g() { x=$(echo hi | cat | cat); echo "$x"; }; g & wait; h() { x='a b'; echo "$x"; }; h & wait"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "hi\na b\n", "stdout: {stdout}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}
