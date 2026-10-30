//! Issue unixwin/rubash#1184 finding 6: the nested-backquote EOF
//! diagnostic. The original finding (extra
//! `syntax error in command substitution` line, wrong line number) is
//! FIXED on master — the first test pins that half as it stands today.
//!
//! The narrowed residual is the expected-token CHAR when a dquote AND a
//! backquote are both open at EOF: GNU names the backquote (innermost),
//! rubash names the dquote.
//!
//! GNU owner: parse.y:5417-5421 read_token_word hands every shellquote
//! opener (`'`, `"`, backtick) to parse_matched_pair; a backtick inside
//! a dquote scan recurses into parse_matched_pair (0, '`', '`',
//! rflags) (parse.y:4157-4160), and because open == close there, the
//! grouping branch (parse.y:4039 `if (open != close)`) never runs —
//! `'` and `"` inside the backquote body are ORDINARY CHARACTERS: they
//! neither open a nesting level nor close the enclosing dquote. A bare
//! backtick closes the body (the `ch == close` count-- at
//! parse.y:3979 precedes the nested-begin branch, so old-style command
//! substitution does not nest). At EOF the innermost failing scan is
//! the body scan, which reports `` unexpected EOF while looking for
//! matching `'` `` at its start_lineno (parse.y:3903-3912), exit 2.
//!
//! Rubash's read-time verdict scanner
//! (src/lexer/continuation.rs `unclosed_input_close_char_posix`) used to
//! push a quote level ON TOP of the backquote, so the verdict named
//! `"`; the scanner now treats quotes as ordinary inside a backquote
//! body (parse.y:4039), and the GNU-expectation pins below run enabled.
//! The rest is pinned as running controls: closed-body shapes
//! (g3/g4/g6/h4), the plain-dquote and backquote-closed verdicts
//! (g1/g5), single-quote body opacity
//! (h1/h2 — parse.y:4000-4007 `open == '\''` continue branch), and the
//! single-diagnostic half of the original finding.
//!
//! All expectations measured against WSL GNU Bash 5.3.20 script files,
//! 2026-10-29, master 25039770.

use std::process::{Command, Stdio};

fn run_rubash_file(script: &str, tag: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("issue1184_f6_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

// ---------------------------------------------------------------------------
// Running controls — behavior already matching GNU on master 25039770.
// ---------------------------------------------------------------------------

/// The original filed repro. The extra
/// `syntax error in command substitution` diagnostic is gone and the
/// line number matches GNU (`line 1`); only the token char still
/// differs (see the ignored pins below).
#[test]
fn nested_backquote_eof_reports_single_diagnostic() {
    let script = r#"echo "3:`echo \`echo \\`echo deep\\\``"
"#;
    let (_stdout, stderr, code) = run_rubash_file(script, "f6single");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    let diags: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("unexpected EOF") || l.contains("syntax error"))
        .collect();
    assert_eq!(
        diags.len(),
        1,
        "GNU prints exactly one diagnostic; got: {stderr:?}"
    );
    assert!(
        !stderr.contains("syntax error in command substitution"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("line 1:"), "stderr: {stderr}");
}

/// Plain unclosed dquote still names `"`.
#[test]
fn unclosed_dquote_names_dquote() {
    let script = "echo \"\n";
    let (_stdout, stderr, code) = run_rubash_file(script, "g1");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("unexpected EOF while looking for matching `\"'"),
        "stderr: {stderr}"
    );
}

/// Backquote opened AND closed inside a dquote, dquote left open: the
/// body scan returned, so GNU's innermost open construct is the dquote.
#[test]
fn closed_backquote_inside_open_dquote_names_dquote() {
    let script = r#"echo "`x`
"#;
    let (_stdout, stderr, code) = run_rubash_file(script, "g5");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("unexpected EOF while looking for matching `\"'"),
        "stderr: {stderr}"
    );
}

/// A fully closed backquote body inside a dquote is COMPLETE input
/// (parse.y:4157 recursion returns into the dquote scan, whose `"`
/// closes it); the comsub body still executes.
#[test]
fn closed_backquote_body_dquote_completes() {
    let script = r#"echo "`x`"
"#;
    let (stdout, stderr, code) = run_rubash_file(script, "g3");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "\n");
    assert!(stderr.contains("x: command not found"), "stderr: {stderr}");
}

/// Single-quote body opacity (parse.y:4000-4007 `open == '\''`
/// continue branch): backticks and dquotes inside a `'...'` body are
/// plain data — `` echo '`x' `` is a complete word.
#[test]
fn single_quote_body_is_opaque_to_other_quotes() {
    let script = "echo '`x'\n";
    let (stdout, stderr, code) = run_rubash_file(script, "h1");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "`x\n");
    assert_eq!(stderr, "");
}

// ---------------------------------------------------------------------------
// GNU-expectation pins for the residual — enabled with the
// src/lexer/continuation.rs backquote-body fix (rubash#1184 f6).
// ---------------------------------------------------------------------------

fn assert_gnu_backquote_verdict(tag: &str, script: &str) {
    let (_stdout, stderr, code) = run_rubash_file(script, tag);
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("unexpected EOF while looking for matching ``'"),
        "stderr: {stderr:?}"
    );
    assert!(
        !stderr.contains("matching `\"'"),
        "dquote must not be the reported token: {stderr:?}"
    );
}

/// `` echo "`x" `` — dquote open, backquote open inside it. GNU
/// (5.3.20, script file): `` line 1: unexpected EOF while looking for
/// matching ``' `` (bytes 60 60 27: open-quote, the backquote, close-quote);
/// rubash used to name `"` here.
#[test]

fn backquote_open_inside_dquote_names_backquote() {
    assert_gnu_backquote_verdict(
        "g2",
        r#"echo "`x"
"#,
    );
}

/// `` echo "`a\`b" `` — an escaped backtick must not close the body
/// (parse.y LEX_PASSNEXT), so the body scan is the innermost failing
/// scan at EOF. GNU names the backquote.
#[test]

fn escaped_backtick_inside_body_still_names_backquote() {
    assert_gnu_backquote_verdict(
        "g7",
        r#"echo "`a\`b"
"#,
    );
}

/// `` echo "`a\\b" `` — an escaped backslash (LEX_PASSNEXT pair) leaves
/// the body open at EOF. GNU names the backquote.
#[test]

fn escaped_backslash_inside_body_still_names_backquote() {
    assert_gnu_backquote_verdict(
        "g8",
        r#"echo "`a\\b"
"#,
    );
}

/// `` echo "`x'y" `` — a single quote inside the backquote body is
/// plain data (no open != close grouping branch), so the verdict is the
/// backquote, not the `'`. GNU names the backquote.
#[test]

fn single_quote_inside_body_is_data_verdict_is_backquote() {
    assert_gnu_backquote_verdict(
        "h3",
        r#"echo "`x'y"
"#,
    );
}

/// The filed repro, full expectation: exactly one diagnostic, line 1,
/// naming the backquote, exit 2.
#[test]

fn nested_backquote_eof_names_backquote() {
    let script = r#"echo "3:`echo \`echo \\`echo deep\\\``"
"#;
    let (_stdout, stderr, code) = run_rubash_file(script, "f6full");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    let diags: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("unexpected EOF") || l.contains("syntax error"))
        .collect();
    assert_eq!(diags.len(), 1, "stderr: {stderr:?}");
    assert!(
        stderr.contains("line 1: unexpected EOF while looking for matching ``'"),
        "stderr: {stderr:?}"
    );
    assert!(
        !stderr.contains("syntax error in command substitution"),
        "stderr: {stderr}"
    );
}
