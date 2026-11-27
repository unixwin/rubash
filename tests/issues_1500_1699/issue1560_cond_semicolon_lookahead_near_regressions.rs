//! Issue unixwin/rubash#1560: when the token that trips the
//! conditional parser is immediately followed (glued, no whitespace)
//! by a `;', GNU's `syntax error near' reports `;' — not the
//! offending token. report_syntax_error's error_token_from_text
//! (parse.y:6772-6808) walks from the READER's position, and the
//! reader has already consumed the glued `;' as its lookahead; the
//! backward walk stops on that break character and reports it alone.
//! A spaced `;' starts a fresh token run and the near stays the
//! offending token's own text. The rule is general to the cond
//! classifier (it is not specific to the `=~` absorption family
//! where it was first measured).
//!
//! GNU owners:
//!  - parse.y:6772-6808 error_token_from_text: anchored at the
//!    reader index, which sits past the glued `;'.
//!  - parse.y:5190/5105: the binary-operator-slot and group-frame
//!    errors whose offending tokens this near belongs to.
//!
//! Verified against GNU Bash 5.3.20 (script-file probes): every
//! expectation below is byte-identical after stripping the
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
    let path = std::env::temp_dir().join(format!("issue1560_{}_{}.sh", tag, std::process::id()));
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
fn issue1560_filed_form_glued_semicolon_after_offender() {
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b)]] && echo T; echo after\n", "filed");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `T', conditional binary operator expected",
            "line 1: syntax error near `;'",
            "line 1: `[[ a =~ (b)]] && echo T; echo after'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1560_glued_semicolon_without_following_space() {
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b)]] && echo T;echo after\n", "glued");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `T', conditional binary operator expected",
            "line 1: syntax error near `;'",
            "line 1: `[[ a =~ (b)]] && echo T;echo after'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1560_double_semicolon_still_reports_single() {
    let (stdout, stderr, code) =
        run_rubash_file("[[ a =~ (b)]] && echo T;; echo after\n", "double");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `T', conditional binary operator expected",
            "line 1: syntax error near `;'",
            "line 1: `[[ a =~ (b)]] && echo T;; echo after'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1560_plain_conditional_binop_offender() {
    // The rule is not absorption-specific: a plain cond binary-op
    // error with a glued `;' follower reports `;' as well.
    let (stdout, stderr, code) = run_rubash_file("[[ a == b c; echo after\n", "plain");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `c'",
            "line 1: syntax error near `;'",
            "line 1: `[[ a == b c; echo after'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1560_group_frame_offender() {
    let (stdout, stderr, code) = run_rubash_file("[[ ( a b; ]]\n", "group");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `b', conditional binary operator expected",
            "line 1: expected `)'",
            "line 1: syntax error near `;'",
            "line 1: `[[ ( a b; ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1560_binary_argument_offender_sibling_form() {
    // The `[[ a ==);x]]` sibling quirk (parser-diag2's known case):
    // the offender is `)', the glued `;' after it owns the near.
    let (stdout, stderr, code) = run_rubash_file("[[ a ==);x]]\n", "sibling");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected argument `)' to conditional binary operator",
            "line 1: syntax error near `;'",
            "line 1: `[[ a ==);x]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1560_end_offender_with_glued_semicolon() {
    // The group-position `((` forms of rubash#1554: the offender is
    // the `]]' itself, and a glued `;' after it takes the near too.
    let (stdout, stderr, code) = run_rubash_file("[[ x && ((a ]]; echo hi\n", "end");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `]]', expected `)'",
            "line 1: expected `)'",
            "line 1: syntax error near `;'",
            "line 1: `[[ x && ((a ]]; echo hi'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1560_spaced_semicolon_keeps_offender_near() {
    // Control: a whitespace-separated `;' is a fresh token run — the
    // near stays the offending token's.
    let (stdout, stderr, code) =
        run_rubash_file("[[ a =~ (b)]] && echo T ; echo after\n", "spaced");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `T', conditional binary operator expected",
            "line 1: syntax error near `T'",
            "line 1: `[[ a =~ (b)]] && echo T ; echo after'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1560_andand_follower_keeps_offender_near() {
    // Control: other followers do not take the near.
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b)]] && echo T && echo x\n", "andand");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `T', conditional binary operator expected",
            "line 1: syntax error near `T'",
            "line 1: `[[ a =~ (b)]] && echo T && echo x'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1560_eof_follower_keeps_offender_near() {
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b)]] && echo T\n", "eof");
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
