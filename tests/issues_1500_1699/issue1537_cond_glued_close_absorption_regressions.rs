//! Issue unixwin/rubash#1537: a `]]` glued directly after a `=~`
//! regex group (`[[ a =~ (b)]]`) is absorbed into GNU's RHS word —
//! read_token_word under PST_REGEXP (parse.y:5443-5461) keeps reading
//! after parse_matched_pair returns, and `]' is not a shell break
//! character, so the conditional has no COND_END at that point and
//! parsing continues with whatever follows (or EOF). rubash's
//! token-level end search treated the glued `]]` as the closer and
//! silently evaluated the conditional (rc 1).
//!
//! GNU owners:
//!  - parse.y:5443-5461 read_token_word + parse_matched_pair: the
//!    regex word is one token; a `]]` inside it is word text.
//!  - parse.y:5013-5029 cond_error / 5223-5227 cond_term: the
//!    diagnostics for the tokens the cond reader meets instead.
//!  - parse.y:6884-6901 report_syntax_error EOF path: the
//!    `unexpected EOF while looking for `]]'' tail.
//!
//! Verified against GNU Bash 5.3.20 (script-file and -c probes):
//! every expectation below is byte-identical after stripping the
//! program-name prefix.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_rubash(args: &[&str], stdin_text: Option<&str>) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin_text.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    if let Some(text) = stdin_text {
        child
            .stdin
            .as_mut()
            .expect("stdin pipe")
            .write_all(text.as_bytes())
            .expect("write stdin");
    }
    let out = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn run_rubash_file(script: &str, tag: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("issue1537_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let result = run_rubash(&[path.to_str().expect("utf8 path")], None);
    let _ = std::fs::remove_file(&path);
    result
}

/// Diagnostic lines with the program-name prefix stripped: every line
/// reduces to its `line N: ...` body, the part GNU pins.
fn diag_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(|line| match line.find("line ") {
            Some(at) => line[at..].to_string(),
            None => line.to_string(),
        })
        .collect()
}

fn expect_diags(stderr: &str, expected: &[&str]) {
    let expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    assert_eq!(diag_lines(stderr), expected);
}

#[test]
fn issue1537_glued_close_after_group_is_unclosed_eof() {
    // `[[ a =~ (b)]]` alone: the RHS word is `(b)]]`, no COND_END
    // remains — GNU reports the unclosed conditional at EOF.
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a =~ (b)]]"], None);
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected EOF while looking for `]]'",
            "line 2: syntax error: unexpected end of file from `[[' command on line 1",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1537_glued_close_then_semicolon_reads_next_token_as_cond_token() {
    // The cond reader continues past the absorbed `]]': the `;' is the
    // offending token inside the conditional expression.
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a =~ (b)]]; echo D"], None);
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `;'",
            "line 1: syntax error near `;'",
            "line 1: `[[ a =~ (b)]]; echo D'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1537_glued_close_group_shape_matrix() {
    // Multiple groups, nested groups, a group mid-word and text after
    // the group all absorb the glued `]]' the same way.
    for input in [
        "[[ a =~ (b)(c)]]; echo D",
        "[[ a =~ ((b))]]; echo D",
        "[[ a =~ x(b)]]; echo D",
        "[[ a =~ (b)c]]; echo D",
        "[[ a =~ b]]; echo D",
    ] {
        let (stdout, stderr, code) = run_rubash(&["-c", input], None);
        assert_eq!(stdout, "", "stdout for {input}");
        expect_diags(
            &stderr,
            &[
                "line 1: syntax error in conditional expression: unexpected token `;'",
                "line 1: syntax error near `;'",
                &format!("line 1: `{input}'"),
            ],
        );
        assert_eq!(code, Some(2), "rc for {input}");
    }
}

#[test]
fn issue1537_glued_close_then_andand_continues_the_conditional() {
    // `&&' after the absorbed `]]' is the conditional's own AND: GNU
    // reads `echo' as the next term and trips on `T'.
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a =~ (b)]] && echo T"], None);
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `T', conditional binary operator expected",
            "line 1: syntax error near `T'",
            "line 1: `[[ a =~ (b)]] && echo T'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1537_glued_close_script_form_reads_next_line() {
    // Script form: the cond reader consumes the NEXT line's first
    // token (`echo') before erroring. The near line is asserted
    // byte-exact now: the glued-`]]' lexer state residue that used to
    // glue it into `echoa' (the following line's tokens lost their
    // leading whitespace) was fixed as rubash#1553.
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b)]]\necho after\n", "script");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `echo'",
            "line 2: syntax error near `echo'",
            "line 2: `echo after'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1537_glued_close_after_multiline_group() {
    // The regex group may itself span lines; a `]]' glued after its
    // `)' on the later line is absorbed the same way, and GNU errors
    // on the word after the following `&&' (file and -c forms alike).
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a =~ (b\n)]] && echo T"], None);
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 2: unexpected token `T', conditional binary operator expected",
            "line 2: syntax error near `T'",
            "line 2: `)]] && echo T'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1537_glued_close_after_quoted_pattern_word() {
    // Absorption is a word property, not a regex-group property: a
    // quoted pattern word also swallows the glued `]]'.
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a == \"b\"]] ; echo D"], None);
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `;'",
            "line 1: syntax error near `;'",
            "line 1: `[[ a == \"b\"]] ; echo D'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1537_absorbed_close_then_real_close_evaluates() {
    // When a standalone `]]' follows later, the conditional is valid:
    // the absorbed `]]' is part of the compared/matched word.
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a =~ (b)]] ]]"], None);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(1));
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a == b]] ]]"], None);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(1));
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a]] ]]"], None);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1537_group_close_glued_close_stays_a_closer() {
    // A `]]' glued after a cond GROUPING `)' is at a token boundary in
    // GNU (the group was delivered as tokens), so it still closes.
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ (a)]]"], None);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ ( a )]]"], None);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    // The spaced regex form is unaffected.
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a =~ (b) ]]"], None);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(1));
}
