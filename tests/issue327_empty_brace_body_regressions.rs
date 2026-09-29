//! Issue rubash#327 (over-lax family, cf. #221): empty or comment-only
//! brace-group bodies were ACCEPTED — `f() { <newline> }`,
//! `f() { <newline> # c <newline> }` and `{ <newline> # c <newline> }`
//! all ran rc=0. GNU rejects every one:
//! `syntax error near unexpected token `}'' (rc 2), the offending line
//! echoed.
//!
//! GNU contract (vendored third_party/bash): parse.y:1196
//! `group_command: '{' compound_list '}'` — compound_list REQUIRES a
//! command; a comment is reader noise (parse.y:3630 read_token discards
//! it) and newlines are separators (parse.y newline_list), so neither
//! constitutes a list. The yacc error names the `}' token at its line and
//! print_offending_line (parse.y:6813-6826) echoes the physical line.
//! Loop/if bodies already enforced this (rubash#221: `do done`); the
//! brace-group and function-body cases did not.
//!
//! Also fixed here: the offending-line echo appended the synthesized `;'
//! raw of a line-break token (`done;' for a bare `done' line) — a
//! line-break Semicolon is the NEWLINE, never on-line text.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

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

/// `f() { <newline> }` — empty function body.
#[test]
fn empty_function_body_rejected() {
    let (stdout, stderr, code) = rubash("f() {\n}\necho after");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 2: syntax error near unexpected token `}'\nbash: -c: line 2: `}'\n"
    );
}

/// `f() { <newline> # comment <newline> }` — comment-only function body.
#[test]
fn comment_only_function_body_rejected() {
    let (stdout, stderr, code) = rubash("f() {\n# c\n}\necho after");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 3: syntax error near unexpected token `}'\nbash: -c: line 3: `}'\n"
    );
}

/// `{ <newline> # comment <newline> }` — comment-only brace group.
#[test]
fn comment_only_brace_group_rejected() {
    let (stdout, stderr, code) = rubash("{\n# c\n}\necho after");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 3: syntax error near unexpected token `}'\nbash: -c: line 3: `}'\n"
    );
}

/// The control from the issue: loop bodies already enforced the rule; the
/// echoed offending line must not carry a stray `;'.
#[test]
fn loop_empty_body_error_has_clean_line_echo() {
    let (stdout, stderr, code) = rubash("while false; do\n# c\ndone\necho after");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "bash: -c: line 3: syntax error near unexpected token `done'\nbash: -c: line 3: `done'\n"
    );
}

/// Real bodies keep working: inline group, multi-line function body,
/// comment-then-command body, nested groups, keyword-form function.
#[test]
fn real_bodies_still_accepted() {
    let (stdout, stderr, code) = rubash(
        "{ :; }\n\
         g() {\n  echo one\n  echo two\n}\n\
         g\n\
         { # c\n :; }\n\
         h() { # real comment\n :; }\n\
         function m { :; }\n\
         echo done",
    );
    assert!(stderr.is_empty(), "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "one\ntwo\ndone\n");
}

/// `{ }` inline (one line, empty) — GNU also rejects it near `}'.
#[test]
fn inline_empty_group_rejected() {
    let (stdout, stderr, code) = rubash("{ }\necho after");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert!(
        stderr.contains("syntax error near unexpected token `}'"),
        "stderr: {stderr}"
    );
}
