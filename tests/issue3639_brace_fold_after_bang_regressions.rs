//! rubash#3639 — trailing unquoted `{ }` after `!` is folded into one
//! word `"{ }"`, where GNU keeps three separate words.
//!
//! GNU anchor: parse.y:3168 `CHECK_FOR_RESERVED_WORD` runs a word through
//! `reserved_word_acceptable (last_read_token)` (parse.y:5899); `!` becomes
//! the BANG token only when read in a reserved-word-acceptable position
//! (word_token_alist parse.y:2378 `{"!", BANG}`). After a plain word
//! (`printf ... !`), `!` is a WORD, so `last_read_token` is WORD and the
//! following `{`/`}` are ordinary words too (read_token_word — `{`/`}` are
//! not in shell_break_chars, syntax.h:30). Rubash's lexer produced `!` as
//! a Keyword unconditionally and recorded it as Keyword in the reader
//! history, so `reserved_word_position()` armed the `{` fold and the
//! lexer folded `{ }` into one Keyword token.
//!
//! The fold also explains the second trigger shape: in `! { } { }` the
//! first `{`'s `}` is rejected as a group close (followed by ` {`), so
//! the first `{` is emitted as a bare Keyword `{`, which makes the `}`
//! a Keyword, which arms the fold for the LAST pair only.
//!
//! Expectations verified against GNU bash 5.3.20 (oracle binary). Each
//! case runs as a script file under the test binary.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3639-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` under rubash and return (stdout, stderr, rc).
fn run_script(script: &str) -> (String, String, i32) {
    let dir = tempdir("case");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let rc = output.status.code().unwrap_or(-1);
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, rc)
}

fn stdout_lines(script: &str) -> Vec<String> {
    let (stdout, _stderr, _rc) = run_script(script);
    stdout.lines().map(str::to_string).collect()
}

fn check_words(script: &str, expected: &[&str]) {
    let got = stdout_lines(script);
    assert_eq!(
        got, expected,
        "script {script:?}: expected words {expected:?}, got {got:?}"
    );
}

#[test]
fn printf_bang_braces_are_three_words() {
    check_words("printf '[%s]\\n' ! { }\n", &["[!]", "[{]", "[}]"]);
}

#[test]
fn command_capital_v_bang_braces_three_lines_no_stderr() {
    let (stdout, stderr, rc) = run_script("command -V ! { }\n");
    assert_eq!(rc, 0, "stderr: {stderr}");
    assert_eq!(
        stdout.lines().count(),
        3,
        "stdout: {stdout:?}, stderr: {stderr:?}"
    );
    assert!(
        stderr.is_empty(),
        "command -V must not diagnose the braces: {stderr:?}"
    );
}

#[test]
fn command_v_bang_braces_prints_three_names() {
    check_words("command -v ! { }\n", &["!", "{", "}"]);
}

#[test]
fn type_bang_braces_rc0_three_lines() {
    let (stdout, stderr, rc) = run_script("type ! { }\n");
    assert_eq!(rc, 0, "stderr: {stderr}");
    assert_eq!(
        stdout.lines().count(),
        3,
        "stdout: {stdout:?}, stderr: {stderr:?}"
    );
}

#[test]
fn printf_bang_not_first_still_three_words() {
    check_words("printf '[%s]\\n' x ! { }\n", &["[x]", "[!]", "[{]", "[}]"]);
}

#[test]
fn printf_two_brace_pairs_stay_separate() {
    check_words(
        "printf '[%s]\\n' ! { } { }\n",
        &["[!]", "[{]", "[}]", "[{]", "[}]"],
    );
}

#[test]
fn printf_three_brace_pairs_stay_separate() {
    check_words(
        "printf '[%s]\\n' ! { } { } { }\n",
        &["[!]", "[{]", "[}]", "[{]", "[}]", "[{]", "[}]"],
    );
}

#[test]
fn for_loop_over_bang_braces_iterates() {
    let (stdout, stderr, rc) = run_script("for w in ! { }; do :; done; echo done\n");
    assert_eq!(rc, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "done", "stdout: {stdout:?}");
}

#[test]
fn control_word_after_close_defeats_fold() {
    // A word after `}` defeats the fold on the base too (guard cell).
    let (stdout, _stderr, _rc) = run_script("command -V ! { } echo\n");
    assert_eq!(stdout.lines().count(), 4, "stdout: {stdout:?}");
}

#[test]
fn control_no_bang_pair_stays_separate() {
    // No `!` at all: the pair stays separate on the base too (guard cell).
    let (stdout, _stderr, _rc) = run_script("command -V { }\n");
    assert_eq!(stdout.lines().count(), 2, "stdout: {stdout:?}");
}

#[test]
fn control_quoted_braces_stay_separate() {
    // Quoted braces are words on the base too (guard cell).
    let (stdout, _stderr, _rc) = run_script("command -V ! '{' '}'\n");
    assert_eq!(stdout.lines().count(), 3, "stdout: {stdout:?}");
}

#[test]
fn control_word_before_last_brace_defeats_fold() {
    // The word before the last `{` is `echo`, not `!`/`}`: no fold on
    // the base either (guard cell).
    check_words(
        "printf '[%s]\\n' ! { } echo { }\n",
        &["[!]", "[{]", "[}]", "[echo]", "[{]", "[}]"],
    );
}

#[test]
fn negated_group_still_folds_and_negates() {
    // A genuine negated group (`!` in command position) must keep
    // folding: the group runs and its status is inverted.
    let (stdout, stderr, rc) = run_script("! { echo inner; }; echo \"rc=$?\"\n");
    assert_eq!(rc, 0, "stderr: {stderr}");
    assert_eq!(stdout, "inner\nrc=1\n", "stdout: {stdout:?}");
}
