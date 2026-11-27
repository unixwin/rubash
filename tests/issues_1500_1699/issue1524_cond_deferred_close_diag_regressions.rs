//! Issue unixwin/rubash#1524: a `=~` group whose `( ... )` closes only
//! on a LATER line than the data `]]` inside it —
//! `[[ a =~ (b ]]` / `) echo after ]]` — lost its syntax error entirely
//! (silent rc=1, execution continued), and the near token of the
//! single-line sibling glued fragments GNU keeps apart
//! (`syntax error near `(b)'` for GNU's `(b'`).
//!
//! GNU owners:
//!  - parse.y:5443-5461 read_token_word under PST_REGEXP: an unquoted
//!    `(' opens parse_matched_pair, so a `]]` inside the open group is
//!    word text, never COND_END; PST_CONDCMD ends only at the
//!    depth-zero `]]'. The lexer port reset its cond state at the data
//!    `]]', which disarmed the tokenize driver's leading_ws capture for
//!    the deferred-close line (the capture reads the pass-start state);
//!    the fragments lost their spacing, the parser merged the rest of
//!    the construct into the RHS word, and the malformed conditional
//!    evaluated as a plain false regex match.
//!  - parse.y:6772-6808 error_token_from_text: the reported token is
//!    the run of non-break characters ending at token_end, and
//!    token_end is shell_input_line_index + 1 — past the offending
//!    reader token at most ONE extra character joins (`(a', `(b',
//!    `()'), never the whole next fragment; a glued `]]' the reader
//!    absorbed into a word (`b]]', `3]]') is part of that word.
//!  - parse.y:5013-5029 cond_error / parse.y:5223-5227 cond_term: the
//!    `unexpected token' / `unexpected argument' messages and lines.
//!
//! Verified against GNU Bash 5.3.20 (script-file, stdin and -c
//! probes): every expectation below is byte-identical after stripping
//! the program-name prefix.

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
    let path = std::env::temp_dir().join(format!("issue1524_{}_{}.sh", tag, std::process::id()));
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

const F1: &str = "echo start\n[[ a =~ (b ]]\n) echo after ]]\n";

#[test]
fn issue1524_deferred_close_script_form_reports_gnu_diagnostics() {
    let (stdout, stderr, code) = run_rubash_file(F1, "f1");
    assert_eq!(stdout, "start\n");
    expect_diags(
        &stderr,
        &[
            "line 2: syntax error in conditional expression: unexpected token `echo'",
            "line 3: syntax error near `echo'",
            "line 3: `) echo after ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1524_deferred_close_stdin_form_reports_gnu_diagnostics() {
    let (stdout, stderr, code) = run_rubash(&[], Some(F1));
    assert_eq!(stdout, "start\n");
    expect_diags(
        &stderr,
        &[
            "line 2: syntax error in conditional expression: unexpected token `echo'",
            "line 3: syntax error near `echo'",
            "line 3: `) echo after ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1524_deferred_close_dash_c_form_reports_gnu_diagnostics() {
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a =~ (b ]]\n) echo after ]]"], None);
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `echo'",
            "line 2: syntax error near `echo'",
            "line 2: `) echo after ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1524_deferred_close_error_aborts_the_script() {
    // GNU reports the error and never reaches `echo done' (the broken
    // form silently evaluated the conditional as false and continued).
    let (stdout, stderr, code) =
        run_rubash_file("[[ a =~ (b ]]\n) echo after ]]\necho done\n", "f2");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `echo'",
            "line 2: syntax error near `echo'",
            "line 2: `) echo after ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1524_deferred_close_paren_alone_line_near_not_glued() {
    // The near token is the offending word alone (`echo'), never glued
    // to the next word (`echoafter').
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b ]]\n)\necho after ]]\n", "f4");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `echo'",
            "line 3: syntax error near `echo'",
            "line 3: `echo after ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1524_deferred_close_glued_paren_offender_is_next_word() {
    // `)echo': the RHS word continues after the matched pair, so the
    // word AFTER the group is the offender (GNU: `after').
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b ]]\n)echo after ]]\n", "f8");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `after'",
            "line 2: syntax error near `after'",
            "line 2: `)echo after ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1524_deferred_close_number_after_paren() {
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b ]]\n) 123 after ]]\n", "f9");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `123'",
            "line 2: syntax error near `123'",
            "line 2: `) 123 after ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1524_deferred_close_offender_on_middle_line() {
    // The offending word sits on its own line between the data `]]'
    // line and the line carrying the real closer.
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b ]]\n) echo after\nmore ]]\n", "f10");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `echo'",
            "line 2: syntax error near `echo'",
            "line 2: `) echo after'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1524_sibling_near_stops_before_closing_paren() {
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a == (b) echo after ]]"], None);
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected argument `(' to conditional binary operator",
            "line 1: syntax error near `(b'",
            "line 1: `[[ a == (b) echo after ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1524_near_token_matrix() {
    // (input, first diagnostic line, near line) — all -c one-liners,
    // GNU Bash 5.3.20 measured. The near token is bounded by
    // error_token_from_text's token_end (reader index + 1 char).
    let cases: &[(&str, &str, &str)] = &[
        // rubash#1125: the whole glued word fragment joins — it is the
        // one character past the `(' token.
        (
            "[[ a == (a ]]",
            "line 1: unexpected argument `(' to conditional binary operator",
            "line 1: syntax error near `(a'",
        ),
        // Only the FIRST character of a longer glued fragment joins.
        (
            "[[ a == (abc ]]",
            "line 1: unexpected argument `(' to conditional binary operator",
            "line 1: syntax error near `(a'",
        ),
        (
            "[[ a == (a)b ]]",
            "line 1: unexpected argument `(' to conditional binary operator",
            "line 1: syntax error near `(a'",
        ),
        // The one extra character may itself be the closing paren.
        (
            "[[ a == ()x ]]",
            "line 1: unexpected argument `(' to conditional binary operator",
            "line 1: syntax error near `()'",
        ),
        // A word offender keeps its glued closing paren whole.
        (
            "[[ a bcd) ]]",
            "line 1: unexpected token `bcd', conditional binary operator expected",
            "line 1: syntax error near `bcd)'",
        ),
        // The walk back over a pattern RHS stops at the operator: the
        // merged RHS word keeps its real leading whitespace.
        (
            "[[ a == x) echo ]]",
            "line 1: syntax error in conditional expression: unexpected token `)'",
            "line 1: syntax error near `x)'",
        ),
        // A glued `]]' absorbed into the offending word stays in it.
        (
            "[[ a b]]",
            "line 1: unexpected token `b]]', conditional binary operator expected",
            "line 1: syntax error near `b]]'",
        ),
        (
            "[[ a 3]]",
            "line 1: unexpected token `3]]', conditional binary operator expected",
            "line 1: syntax error near `3]]'",
        ),
        (
            "[[ a =~ b c]]",
            "line 1: syntax error in conditional expression: unexpected token `c]]'",
            "line 1: syntax error near `c]]'",
        ),
        // rubash#1353 break-led fragments: the report starts at the
        // break character and takes one more character.
        (
            "[[ 5&3]]",
            "line 1: unexpected token `&', conditional binary operator expected",
            "line 1: syntax error near `&3'",
        ),
        (
            "[[ -n x&y ]]",
            "line 1: syntax error in conditional expression: unexpected token `&'",
            "line 1: syntax error near `&y'",
        ),
    ];
    for (input, first, near) in cases {
        let (stdout, stderr, code) = run_rubash(&["-c", input], None);
        assert_eq!(stdout, "", "stdout for {input}");
        expect_diags(&stderr, &[first, near, &format!("line 1: `{input}'")]);
        assert_eq!(code, Some(2), "rc for {input}");
    }
}

#[test]
fn issue1524_valid_deferred_close_forms_stay_silent() {
    // A group that closes on a later line is a valid regex word: the
    // conditional is simply false, rc=1, no diagnostics. (The glued
    // `)]]' spelling of this form is NOT valid -- the `]]' is
    // absorbed into the regex word exactly like the single-line
    // form, and GNU errors on the following `T': that spelling is
    // pinned with GNU expectations in the issue1537 file.)
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a =~ (b ]\n) ]] && echo T"], None);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(1));
}

/// Adjacent divergence to #1524, fixed by rubash#1537: GNU's regex
/// word absorbs a `]]' glued directly after the group (`(b)]]` is
/// the whole RHS word), so no COND_END remains and GNU reports an
/// unclosed conditional; rubash's token-level end search treated
/// that `]]' as the closer and evaluated the conditional.
#[test]
fn issue1524_adjacent_glued_close_after_group_is_unclosed_in_gnu() {
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
