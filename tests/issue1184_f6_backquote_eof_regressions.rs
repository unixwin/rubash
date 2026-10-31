//! Issue unixwin/rubash#1184 finding 6 (f6 v2): the nested-backquote EOF
//! diagnostic, under the corrected two-layer design recorded on #1184
//! (comment-6522658267). This file REPLACES
//! tests/issue1184_f6_backquote_eof_regressions.rs when the fix lands.
//!
//! GNU's model is two-layer (parse.y): a backquote body is DELIMITED
//! atomically — the matcher (parse.y:4157-4160 recursing from
//! read_token_word's shellquote arm) runs parse_matched_pair with
//! open == close == '`', so the grouping branch (parse.y:4039
//! `if (open != close)`) never runs inside the body: `'` and `"` there
//! are ordinary characters, they neither nest nor close anything, and a
//! bare backtick closes the body (the `ch == close` count-- at
//! parse.y:3979 precedes the nested-begin branch — old-style
//! substitution does not nest). EOF mid-gathering therefore reports the
//! BACKQUOTE at its start_lineno (parse.y:3903-3912). The extracted
//! body is then PARSED as its own input: quotes group normally inside
//! it, and a body that is itself unclosed takes the body-parse
//! diagnostic — `<name>: command substitution: line N: unexpected EOF
//! while looking for matching \`X'` — with the substitution yielding
//! empty and status 2 while the outer command line runs on.
//!
//! Layer 1 of the fix (src/lexer/continuation.rs, captain-exclusive —
//! lands only under the owner's per-case authorization): in both
//! verdict scans (`unclosed_input_close_char_posix` and its resumable
//! mirror `close_char_residuals_advance`), a backquote delimiter on the
//! stack top suppresses every push arm below it, so the body
//! contributes no entries to the outer stack. Layer 2
//! (src/executor/command_substitution.rs): a backquote body whose own
//! verdict scan reports a pending construct takes the
//! `command substitution:` diagnostic path instead of executing
//! leniently.
//!
//! Byte note (od-verified against GNU bash 5.3.20): the diagnostic
//! wording is `matching \`` + the delimiter char + `'` — for the
//! backquote itself that is `matching ` followed by bytes 60 60 27.
//! The pre-v2 copy of this file asserted the stale needle
//! `matching \`'` (60 27) for the backquote, which can never match
//! GNU's output; the assertions below use the true bytes.
//!
//! All expectations measured against GNU bash 5.3.20 script files
//! (local oracle build), master bbd95e37, 2026-10-30. Controls are
//! shapes already GNU-true on that master; acceptance pins are the
//! f6 cells (red on that master, green under the two-layer fix).

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

fn diagnostic_lines(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|l| l.contains("unexpected EOF") || l.contains("syntax error"))
        .collect()
}

// ---------------------------------------------------------------------------
// Running controls — behavior already matching GNU on master bbd95e37.
// ---------------------------------------------------------------------------

/// The original filed repro: exactly one diagnostic, line 1. (The CHAR
/// it names is pinned by nested_backquote_eof_names_backquote below.)
#[test]
fn nested_backquote_eof_reports_single_diagnostic() {
    let script = r#"echo "3:`echo \`echo \\`echo deep\\\``"
"#;
    let (_stdout, stderr, code) = run_rubash_file(script, "f6single");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    let diags = diagnostic_lines(&stderr);
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
/// body was delimited completely, so the verdict falls back to the
/// dquote below it.
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

/// A fully closed backquote body inside a dquote is COMPLETE input;
/// the comsub body still executes.
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

/// quote.tests:117 — quotes group normally INSIDE an extracted body:
/// `` echo `echo '${'` `` prints `${`. (The shape that falsified the
/// #2159 single-rule guard in PR #2209: body quotes must NOT be made
/// transparent at the delimiting layer.)
#[test]
fn body_quotes_group_normally_quote_tests_117() {
    let script = "echo `echo '${'`\n";
    let (stdout, stderr, code) = run_rubash_file(script, "s4");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "${\n");
    assert_eq!(stderr, "");
}

/// quote.tests:100-102 verbatim — a backquote body spanning physical
/// lines (backslash-newline inside the body's own single quotes).
#[test]
fn multiline_body_quote_tests_100_102() {
    let script = "a=`echo 'a b c' | sed 's/ /\\\\\n/g' | grep 'b'`\necho $a\n";
    let (stdout, stderr, code) = run_rubash_file(script, "m1");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "b\n");
    assert_eq!(stderr, "");
}

/// quote.tests:103-105 verbatim — same span, unquoted assignment echo.
#[test]
fn multiline_body_quote_tests_103_105() {
    let script = "a=`echo 'a b c' | sed 's/ /\\\\\n/g'`\necho \"$a\"\n";
    let (stdout, stderr, code) = run_rubash_file(script, "m2");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a\nb\nc\n");
    assert_eq!(stderr, "");
}

/// EOF inside a backquote gather that sits inside a `$(` body already
/// names the backquote on master (the comsub route scans the unit
/// atomically); the layer-1 gate must not regress it.
#[test]
fn backquote_eof_inside_dollar_paren_names_backquote() {
    let script = "echo $(`echo 'a\n";
    let (_stdout, stderr, code) = run_rubash_file(script, "t5");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("unexpected EOF while looking for matching ``'"),
        "stderr: {stderr:?}"
    );
}

/// Closed backquote inside an unclosed `$(`: the verdict falls to the
/// enclosing `)` (EOF-line report, line 2).
#[test]
fn closed_backquote_inside_open_dollar_paren_names_paren() {
    let script = "echo $(`x`\n";
    let (_stdout, stderr, code) = run_rubash_file(script, "t6");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(
        stderr.contains("line 2: unexpected EOF while looking for matching `)'"),
        "stderr: {stderr:?}"
    );
}

/// Layer-2 boundary control: a body left open by a KEYWORD (`if`
/// without `fi`) carries no matched-pair pending construct, so it
/// stays with the body parser's own diagnostic family — GNU:
/// `command substitution: line 2: syntax error: unexpected end of
/// file from \`if' command on line 1`. The verdict pre-check must not
/// intercept this shape. Already GNU-true on master.
#[test]
fn unclosed_keyword_body_keeps_parser_diagnostic() {
    let script = "x=`if true; then echo x`; echo \"rc=$? x=<$x>\"\n";
    let (stdout, stderr, code) = run_rubash_file(script, "u8");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc=2 x=<>\n");
    assert!(
        stderr.contains("command substitution: line 2: syntax error: unexpected end of file"),
        "stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("from `if' command on line 1"),
        "stderr: {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// Acceptance pins — layer 1 (reader verdict names the backquote).
// Red on master bbd95e37; green under the layer-1 gate.
// ---------------------------------------------------------------------------

fn assert_gnu_backquote_verdict(tag: &str, script: &str, wrong_needles: &[&str]) {
    let (_stdout, stderr, code) = run_rubash_file(script, tag);
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert_eq!(diagnostic_lines(&stderr).len(), 1, "stderr: {stderr:?}");
    // GNU's bytes: "matching " + 60 60 27.
    assert!(
        stderr.contains("unexpected EOF while looking for matching ``'"),
        "stderr: {stderr:?}"
    );
    for needle in wrong_needles {
        assert!(!stderr.contains(needle), "wrong token named: {stderr:?}");
    }
}

/// `` echo "`x" `` — dquote open, backquote open inside it, and the
/// trailing `"` is body text. GNU names the backquote.
#[test]
fn backquote_open_inside_dquote_names_backquote() {
    assert_gnu_backquote_verdict("g2", "echo \"`x\"\n", &["matching `\"'"]);
}

/// `` echo "`a\`b" `` — an escaped backtick must not close the body
/// (parse.y LEX_PASSNEXT). GNU names the backquote.
#[test]
fn escaped_backtick_inside_body_still_names_backquote() {
    assert_gnu_backquote_verdict("g7", "echo \"`a\\`b\"\n", &["matching `\"'"]);
}

/// `` echo "`a\\b" `` — an escaped backslash pair leaves the body
/// open at EOF. GNU names the backquote.
#[test]
fn escaped_backslash_inside_body_still_names_backquote() {
    assert_gnu_backquote_verdict("g8", "echo \"`a\\\\b\"\n", &["matching `\"'"]);
}

/// `` echo "`x'y" `` — a single quote inside the body is plain data.
/// GNU names the backquote, not the `'`.
#[test]
fn single_quote_inside_body_is_data_verdict_is_backquote() {
    assert_gnu_backquote_verdict("h3", "echo \"`x'y\"\n", &["matching `''"]);
}

/// `` echo `echo 'a `` (EOF) — EOF mid-gathering, no dquote wrapper:
/// the quote inside the gathering body is not a pending construct.
/// GNU names the backquote.
#[test]
fn eof_mid_gathering_squote_names_backquote() {
    assert_gnu_backquote_verdict("t1", "echo `echo 'a\n", &["matching `''"]);
}

/// `` echo `echo "a `` (EOF) — same with a dquote inside the body.
/// GNU names the backquote.
#[test]
fn eof_mid_gathering_dquote_names_backquote() {
    assert_gnu_backquote_verdict("t3", "echo `echo \"a\n", &["matching `\"'"]);
}

/// The filed repro, full expectation: exactly one diagnostic, line 1,
/// naming the backquote (bytes 60 60 27), exit 2.
#[test]
fn nested_backquote_eof_names_backquote() {
    let script = r#"echo "3:`echo \`echo \\`echo deep\\\``"
"#;
    let (_stdout, stderr, code) = run_rubash_file(script, "f6full");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert_eq!(diagnostic_lines(&stderr).len(), 1, "stderr: {stderr:?}");
    assert!(
        stderr.contains("line 1: unexpected EOF while looking for matching ``'"),
        "stderr: {stderr:?}"
    );
    assert!(
        !stderr.contains("syntax error in command substitution"),
        "stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Acceptance pins — layer 2 (an unclosed BODY takes the
// `command substitution:` diagnostic; the outer line runs on, the
// substitution yields empty, its status is 2). Red on master
// bbd95e37 (the reader verdict pre-empts or the route misfires);
// green under layers 1+2.
// ---------------------------------------------------------------------------

fn assert_unclosed_body_diagnostic(
    tag: &str,
    script: &str,
    expected_stdout: &str,
    expected_diag: &str,
) {
    let (stdout, stderr, code) = run_rubash_file(script, tag);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, expected_stdout, "stderr: {stderr:?}");
    assert_eq!(diagnostic_lines(&stderr).len(), 1, "stderr: {stderr:?}");
    assert!(stderr.contains(expected_diag), "stderr: {stderr:?}");
}

/// `` x=`echo 'a` `` — the body is delimited fine; the BODY is
/// unclosed. GNU: `command substitution: line 1: ... matching \`''`,
/// x empty, the assignment's status is 2, the script continues.
#[test]
fn unclosed_squote_body_takes_comsub_diagnostic() {
    assert_unclosed_body_diagnostic(
        "u1",
        "x=`echo 'a`; echo \"rc=$? x=<$x>\"\n",
        "rc=2 x=<>\n",
        "command substitution: line 1: unexpected EOF while looking for matching `''",
    );
}

/// `` x=`echo "a` `` — same, naming the body's dquote.
#[test]
fn unclosed_dquote_body_takes_comsub_diagnostic() {
    assert_unclosed_body_diagnostic(
        "u2",
        "x=`echo \"a`; echo \"rc=$? x=<$x>\"\n",
        "rc=2 x=<>\n",
        "command substitution: line 1: unexpected EOF while looking for matching `\"'",
    );
}

/// The diagnostic line is the line of the command containing the
/// substitution (the body's start line), not a body-relative line.
#[test]
fn unclosed_body_diagnostic_line_is_command_line() {
    assert_unclosed_body_diagnostic(
        "u1l3",
        "echo first\necho second\nx=`echo 'a`; echo \"rc=$? x=<$x>\"\n",
        "first\nsecond\nrc=2 x=<>\n",
        "command substitution: line 3: unexpected EOF while looking for matching `''",
    );
}

/// An unclosed body inside a dquote: the enclosing command still
/// prints (with the substitution empty).
#[test]
fn unclosed_body_inside_dquote_still_prints() {
    assert_unclosed_body_diagnostic(
        "u4",
        "echo \"v=`echo 'a`\"\n",
        "v=\n",
        "command substitution: line 1: unexpected EOF while looking for matching `''",
    );
}

/// An unclosed `$(` INSIDE the body: matched-pair pendings other than
/// quotes take the same family, at the body's EOF line (the `$(`
/// report is the yyerror/EOF-line form: body line 1 + 1 = line 2).
#[test]
fn unclosed_dollar_paren_body_takes_comsub_diagnostic() {
    assert_unclosed_body_diagnostic(
        "u5",
        "x=`echo $(x`; echo \"rc=$? x=<$x>\"\n",
        "rc=2 x=<>\n",
        "command substitution: line 2: unexpected EOF while looking for matching `)'",
    );
}

/// An unclosed `${` inside the body names `}` at its open line.
#[test]
fn unclosed_dollar_brace_body_takes_comsub_diagnostic() {
    assert_unclosed_body_diagnostic(
        "u6",
        "x=`echo ${x`; echo \"rc=$? x=<$x>\"\n",
        "rc=2 x=<>\n",
        "command substitution: line 1: unexpected EOF while looking for matching `}'",
    );
}

/// The heredoc route reaches body execution without any reader
/// verdict over the body text (heredoc bodies are opaque to the
/// reader scan): the same diagnostic must come out of the expansion
/// path. GNU: stdout `x=`, the line-1 `command substitution:`
/// diagnostic naming `'`.
#[test]
fn unclosed_body_in_heredoc_expansion_takes_comsub_diagnostic() {
    assert_unclosed_body_diagnostic(
        "l2",
        "cat <<EOF\nx=`echo 'a`\nEOF\n",
        "x=\n",
        "command substitution: line 1: unexpected EOF while looking for matching `''",
    );
}

/// Design §2.3 (added under the landing brief): a body containing an
/// unclosed NESTED backquote — after \` restoration the inner
/// backquote opens a nested substitution whose own body never
/// closes. The verdict scan over the restored body reports the
/// nested backquote, and the one diagnostic format composes
/// `matching ` + bytes 60 60 27 with no special case.
/// Oracle-verified byte-for-byte against GNU bash 5.3.20.
#[test]
fn unclosed_nested_backquote_body_names_backquote() {
    assert_unclosed_body_diagnostic(
        "nested",
        "x=`echo \\`echo 'a`; echo \"rc=$? x=<$x>\"\n",
        "rc=2 x=<>\n",
        "command substitution: line 1: unexpected EOF while looking for matching ``'",
    );
}
