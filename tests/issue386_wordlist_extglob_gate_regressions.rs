//! rubash#386: with the extglob shopt OFF (the default), GNU rejects an
//! extended-glob group in a `for`/`select` word list
//! (`select x in a @(b|c)` -> `syntax error near unexpected token `('`,
//! rc 2) while rubash accepted it and split the group into menu/loop
//! words (`@`, `(`, `b`, `c`, `)`). A plain paren (`for x in a (b)`) is
//! the same grammar slot and was also silently accepted.
//!
//! GNU spec:
//! - parse.y:1037-1054 (for_command/select_command wordlist): the list
//!   admits only WORD tokens; a `(' there is the yacc error
//!   `syntax error near unexpected token `(''.
//! - parse.y:5466 read_token_word absorbs the pattern group into the
//!   word ONLY while the extglob shopt is on (extended_glob), so with
//!   the gate closed the `(' survives as its own token — the scanner's
//!   gated-split marker (rubash#131) already models this for
//!   simple-command and case-pattern positions; the word-list loops
//!   (parser/for_command.rs, parser/select_command.rs) now reject the
//!   bare `(' token the same way instead of skipping/joining it.
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash) script-file probes
//! (target/issue-suites/results/wt37-gapfix1/, 2026-10-02).

use std::process::Command;

fn rubash(script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (output.stdout, output.stderr, output.status.code())
}

fn assert_rejected(script: &str, offending_line: &str) {
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(code, Some(2), "rc for {script:?}");
    assert!(stdout.is_empty(), "stdout for {script:?}: {stdout:?}");
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "stderr for {script:?}: {stderr}"
    );
    assert!(
        stderr.contains(offending_line),
        "offending line not echoed for {script:?}: {stderr}"
    );
}

#[test]
fn select_word_list_rejects_extglob_group_when_gate_closed() {
    // The #386 reproducer.
    assert_rejected(
        "select x in a @(b|c); do :; done < /dev/null",
        "select x in a @(b|c); do :; done < /dev/null",
    );
}

#[test]
fn for_word_list_rejects_extglob_groups_when_gate_closed() {
    assert_rejected(
        "for x in a @(b|c); do :; done",
        "for x in a @(b|c); do :; done",
    );
    assert_rejected(
        "for x in a +(b) !(c) ?(d) *(e); do :; done",
        "for x in a +(b) !(c) ?(d) *(e); do :; done",
    );
    // An escaped operator still leaves the `(' a bare token.
    assert_rejected(
        "for x in a \\@(b); do :; done",
        "for x in a \\@(b); do :; done",
    );
}

#[test]
fn bare_paren_in_word_list_is_the_same_error() {
    assert_rejected("for x in a (b); do :; done", "for x in a (b); do :; done");
    assert_rejected(
        "select x in a (b); do :; done < /dev/null",
        "select x in a (b); do :; done < /dev/null",
    );
}

#[test]
fn extglob_on_word_lists_still_parse_and_expand() {
    // With the shopt on, the group is ONE word (already correct before
    // this fix — the gate must not over-reject).
    let (stdout, stderr, code) =
        rubash("shopt -s extglob\nfor x in a @(b|c); do echo \"[$x]\"; done");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"[a]\n[@(b|c)]\n");
    assert!(stderr.is_empty(), "{stderr:?}");
}

#[test]
fn quoted_and_plain_word_lists_unchanged() {
    let (stdout, stderr, code) = rubash("for x in \"@(b)\" 'c(1)' plain; do echo \"[$x]\"; done");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"[@(b)]\n[c(1)]\n[plain]\n");
    assert!(stderr.is_empty(), "{stderr:?}");
    let (stdout, stderr, code) = rubash("for x in a; do echo \"[$x]\"; done");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"[a]\n");
    assert!(stderr.is_empty(), "{stderr:?}");
}
