//! Issue rubash#332: `shopt -u extglob` mid-script left the LATER
//! `echo +(a|b)` reporting the stray `)' instead of GNU's `(' operator
//! error, and the related `(' -in-argument-position family misnamed its
//! offending token.
//!
//! GNU contract (vendored third_party/bash):
//! - parse.y:5464-5490 (read_token_word) gates extglob pattern groups on
//!   the LIVE `extended_glob' variable, which the shopt builtin updates as
//!   commands execute (builtins/shopt.def) — bash reads a command, runs
//!   it, then parses the next, so `shopt -u extglob' closes the gate for
//!   every later line (rubash#131's PARSE_EXTENDED_GLOB line-loop mirror
//!   already did this; the folded-vs-split tokens proved the gate itself
//!   was correct).
//! - parse.y:1054 `function_def: WORD '(' ')' newline_list function_body':
//!   the LALR parser shifts a `(' after EXACTLY ONE WORD at command
//!   position as a function-head candidate — pending `!' inversion still
//!   allows the shift (`! +(a|b)' errors at `a'), an assignment or
//!   redirection prefix does not (`x=1 y=2 +(a|b)', `>f (a)' error at the
//!   `(' itself) — and only `)' may follow the shifted `('. With two or
//!   more words the grammar has no `(' production: the error names the
//!   `(' itself.
//! - parse.y:6724 yyerror → parse.y:6833 report_syntax_error: the FIRST
//!   error aborts the rest of the input, so `echo +(a|b)' with the gate
//!   closed names `(', never a later stray `)'.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28, wt18/misc17
//! target/misc17/{xt*,w*}.sh artifacts).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
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

/// The issue's own reproducer: `shopt -u extglob` mid-script must close
/// the parse-time gate, so the later `echo +(a|b)' is GNU's `(' operator
/// error (the earlier still-enabled line prints its glob literal first,
/// and `echo done' never runs).
#[test]
fn shopt_unset_closes_parse_gate_for_later_lines() {
    let (stdout, stderr, code) =
        rubash("shopt -s extglob\necho +(a|b)\nshopt -u extglob\necho +(a|b)\necho done");
    assert_eq!(stdout, "+(a|b)\n");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 4: syntax error near unexpected token `('\nbash: -c: line 4: `echo +(a|b)'\n"
    );
}

/// Same flip on one logical line (net disable): the `(' error is reported
/// at the tail command.
#[test]
fn shopt_unset_single_line_still_closes_gate() {
    let (stdout, stderr, code) = rubash("shopt -s extglob; shopt -u extglob; echo +(a|b)");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 1: syntax error near unexpected token `('\nbash: -c: line 1: `shopt -s extglob; shopt -u extglob; echo +(a|b)'\n"
    );
}

/// Gate OFF from the start (no shopt anywhere): extglob-off `+(a|b)' is
/// the same `(' error.
#[test]
fn extglob_off_from_start_names_open_paren() {
    let (stdout, stderr, code) = rubash("echo +(a|b)");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 1: syntax error near unexpected token `('\nbash: -c: line 1: `echo +(a|b)'\n"
    );
}

/// A word ending in a glob character before the group: `+x(a|b)' splits
/// into `+x' and the `(' operator (parse.y:5466 ends the word when the
/// gate is closed).
#[test]
fn extglob_off_word_glob_tail_names_open_paren() {
    let (stdout, stderr, code) = rubash("echo +x(a|b)");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 1: syntax error near unexpected token `('\nbash: -c: line 1: `echo +x(a|b)'\n"
    );
}

/// Two words before the `(' — no function-head candidate: error at the
/// `(' itself.
#[test]
fn two_words_before_paren_names_open_paren() {
    let (stdout, stderr, code) = rubash("echo a (b)");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 1: syntax error near unexpected token `('\nbash: -c: line 1: `echo a (b)'\n"
    );
}

/// Assignment prefix commits the parse to simple_command: the func-head
/// shift is unavailable, so the error names the `(' (GNU
/// `x=1 y=2 +(a|b)', parse.y:1054).
#[test]
fn assignment_prefix_blocks_function_head_shift() {
    let (stdout, stderr, code) = rubash("x=1 y=2 +(a|b)");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 1: syntax error near unexpected token `('\nbash: -c: line 1: `x=1 y=2 +(a|b)'\n"
    );
}

/// A leading assignment command followed by the two-word form — still the
/// `(' error at the tail.
#[test]
fn assignment_then_two_words_names_open_paren() {
    let (stdout, stderr, code) = rubash("x=1; echo +(a|b)");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 1: syntax error near unexpected token `('\nbash: -c: line 1: `x=1; echo +(a|b)'\n"
    );
}

/// Pending `!' inversion does NOT block the function-head shift: the
/// error names the token AFTER the `(' (`a').
#[test]
fn bang_prefix_keeps_function_head_shift() {
    let (stdout, stderr, code) = rubash("! echo (a)");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 1: syntax error near unexpected token `a'\nbash: -c: line 1: `! echo (a)'\n"
    );
}

/// Inside a function body the same two-word rule applies at body parse
/// time (`f() { echo +(a|b); }').
#[test]
fn function_body_two_words_names_open_paren() {
    let (stdout, stderr, code) = rubash("f() { echo +(a|b); }\necho after");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 1: syntax error near unexpected token `('\nbash: -c: line 1: `f() { echo +(a|b); }'\n"
    );
}

/// yyerror aborts at the FIRST error: a later stray `)' never overwrites
/// the `(' report (`echo +(a|b) )').
#[test]
fn first_syntax_error_wins_over_later_stray_close() {
    let (stdout, stderr, code) = rubash("echo +(a|b) )");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 1: syntax error near unexpected token `('\nbash: -c: line 1: `echo +(a|b) )'\n"
    );
}

/// The gate follows the runtime shopt back ON after a mid-script unset
/// (parse.y:5466 reads `extended_glob' live): the re-enabled `@(a|b)' is
/// a glob again and runs.
#[test]
fn gate_reopens_after_reenable() {
    let (stdout, stderr, code) =
        rubash("shopt -s extglob\nshopt -u extglob\nshopt -s extglob\necho @(a|b)\necho rc0");
    assert_eq!(stdout, "@(a|b)\nrc0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// A body folded while the gate was OPEN keeps executing after the gate
/// closed at runtime (Token::extglob_gate stamps, parse.y:1054 note):
/// `f' still prints the glob literal.
#[test]
fn gate_stamp_preserves_open_gate_body() {
    let (stdout, stderr, code) = rubash("shopt -s extglob\nf() { echo +(a|b); }\nf\necho after");
    assert_eq!(stdout, "+(a|b)\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
