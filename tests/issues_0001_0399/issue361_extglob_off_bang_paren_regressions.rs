//! Issue rubash#361 — bare `!(*.mp3|*.wmv)` at command position with
//! extglob OFF was lexed as an extglob pattern word and rejected
//! (`syntax error near unexpected token `*.mp3'`); GNU parses it as the
//! BANG reserved word negating the subshell `(*.mp3|*.wmv)` and accepts
//! (`bash -n` rc 0; at runtime the subshell's two command-not-found
//! failures are negated to rc 0).
//!
//! GNU anchor: parse.y read_token_word's pattern-group consumption is
//! gated on the live `extended_glob` (parse.y:5466); with the gate closed
//! the word scan ends at `(`, so the token is the bare word `!`, and
//! CHECK_FOR_RESERVED_WORD (parse.y:3168-3181) converts it to BANG at
//! command position. (GNU 5.3.0 verified on WSL: `shopt -s extglob`
//! mid-script — same line, later line, or `-c` — does NOT open the
//! parse-time gate; only shell-startup `-O extglob` does. rubash's gate
//! tracking already matches that, rubash#131/#332.)
//!
//! Fix: the scanner's `!(` branch emits the Keyword `!` when the gate is
//! closed AND the position accepts a reserved word, leaving `(` to lex
//! as the subshell opener. Gate open (`-O extglob`): unchanged extglob
//! word. Non-command position with gate closed (`echo !(*.c)`):
//! unchanged — GNU and rubash both report `syntax error near unexpected
//! token `('`.
//!
//! Verification: full 2164-snippet ShellCheck corpus rerun — 2164/2164
//! rc parity (Parser_1985 fixed; no new divergences). Suites extglob
//! 16=16, extglob2 0, extglob3 0, glob 44=44 — rb.out byte-identical to
//! same-harness master-tree reruns. issue332/issue349_350 extglob
//! regression tests all pass.

use std::process::Command;

fn rubash_n(args: &[&str], script: &str) -> Option<i32> {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .arg("-n")
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -n")
        .status
        .code()
}

fn rubash_run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's probe: bare `!(...)` at command position, extglob OFF,
/// parses (rc 0 under -n).
#[test]
fn bang_paren_command_position_parses() {
    assert_eq!(rubash_n(&[], "!(*.mp3|*.wmv)"), Some(0));
}

/// At runtime it is `!` + subshell: the command-not-found failures are
/// negated, rc 0 (GNU prints both diagnostics).
#[test]
fn bang_paren_command_position_runs_negated() {
    let (_stdout, stderr, code) = rubash_run("!(*.mp3|*.wmv)");
    assert!(
        stderr.contains("*.mp3: command not found"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("*.wmv: command not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

/// Gate open via shell-startup `-O extglob`: `!(` stays an extglob word
/// (matches a filename or fails at execution, not at parse).
#[test]
fn bang_paren_with_extglob_on_is_pattern_word() {
    assert_eq!(rubash_n(&["-O", "extglob"], "!(*.mp3|*.wmv)"), Some(0));
}

/// Non-command position with the gate closed still errors (GNU:
/// `syntax error near unexpected token `('`).
#[test]
fn bang_paren_operand_position_still_rejected() {
    assert_eq!(rubash_n(&[], "echo !(*.c)"), Some(2));
}

/// `shopt -s extglob` at runtime does not open the PARSE-time gate in
/// GNU 5.3.0 — same line or later lines both stay errors.
#[test]
fn runtime_shopt_does_not_open_parse_gate() {
    assert_eq!(rubash_n(&[], "shopt -s extglob; echo !(*.c)"), Some(2));
    assert_eq!(rubash_n(&[], "shopt -s extglob\necho !(*.c)"), Some(2));
}

/// Spelled-out `! (subshell)` keeps working in every command position.
#[test]
fn bang_subshell_forms() {
    let (stdout, stderr, code) = rubash_run("! (true) ; echo \"rc=$?\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(code, Some(0));
    let (stdout, stderr, code) = rubash_run("if ! (false); then echo NEG; fi");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "NEG\n");
    assert_eq!(code, Some(0));
}
