//! Issue unixwin/rubash#1554: an unclosed `((` in GROUP position
//! inside a conditional (`[[ x && ((a ]]`) was pre-empted by the
//! script driver's unclosed-paren route, which printed
//! `unexpected EOF while looking for matching `)'' — the route's
//! parser disproof only ran for command-position verdicts, and the
//! verdict for a cond-internal `((` is command=false. GNU's cond
//! reader owns the diagnostic: cond_term's group frames each report
//! one `expected `)'` line (the innermost in the combined
//! `unexpected token `]]', expected `)'` form), followed by the
//! reader-level near report.
//!
//! GNU owners:
//!  - parse.y:5074-5247 cond_term/cond_or: the group grammar and its
//!    per-frame `expected `)'` reports (5100-5109).
//!  - parse.y:5013-5029 cond_error + 6833-6901 report_syntax_error:
//!    the specific messages, then `syntax error near' from
//!    error_token_from_text (6772-6808) — whose backward text walk
//!    stops ON a non-whitespace break character (`&]]') and crosses
//!    glued fragments (`(]]').
//!  - parse.y:3780: the arithmetic-command special case for `((` is
//!    gated OFF inside a conditional — a cond `((` is two group opens.
//!
//! Verified against GNU Bash 5.3.20 (script-file probes): every
//! expectation below is byte-identical after stripping the
//! program-name prefix. The `;`-follower near forms of this family
//! are pinned by the issue1560 regression file (the reader-lookahead
//! `;' report is that issue's phenomenon).

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
    let path = std::env::temp_dir().join(format!("issue1554_{}_{}.sh", tag, std::process::id()));
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

const COMBINED_ONE_BARE_NEAR_END: &[&str] = &[
    "line 1: unexpected token `]]', expected `)'",
    "line 1: expected `)'",
    "line 1: syntax error near `]]'",
];

#[test]
fn issue1554_filed_form_and_position_spaced() {
    let (stdout, stderr, code) = run_rubash_file("[[ x && ((a ]]\n", "filed");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            COMBINED_ONE_BARE_NEAR_END[0],
            COMBINED_ONE_BARE_NEAR_END[1],
            COMBINED_ONE_BARE_NEAR_END[2],
            "line 1: `[[ x && ((a ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_or_position() {
    let (stdout, stderr, code) = run_rubash_file("[[ x || ((a ]]\n", "or");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            COMBINED_ONE_BARE_NEAR_END[0],
            COMBINED_ONE_BARE_NEAR_END[1],
            COMBINED_ONE_BARE_NEAR_END[2],
            "line 1: `[[ x || ((a ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_bang_position() {
    let (stdout, stderr, code) = run_rubash_file("[[ ! ((a ]]\n", "bang");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            COMBINED_ONE_BARE_NEAR_END[0],
            COMBINED_ONE_BARE_NEAR_END[1],
            COMBINED_ONE_BARE_NEAR_END[2],
            "line 1: `[[ ! ((a ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_glued_close_word_absorbs_into_group_word() {
    // `((a]]`: the `]]` is absorbed into the inner word, so the inner
    // expression completes and the binary-operator slot meets the
    // newline; both group frames then report a bare `expected `)'`.
    let (stdout, stderr, code) = run_rubash_file("[[ x && ((a]]\n", "glued");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `newline', conditional binary operator expected",
            "line 1: expected `)'",
            "line 1: expected `)'",
            "line 1: syntax error near `((a]]'",
            "line 1: `[[ x && ((a]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_eof_right_after_group_word() {
    let (stdout, stderr, code) = run_rubash_file("[[ x && ((a\n", "eof");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `newline', conditional binary operator expected",
            "line 1: expected `)'",
            "line 1: expected `)'",
            "line 1: syntax error near `((a'",
            "line 1: `[[ x && ((a'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_dparen_nested_in_plain_group() {
    // Three unclosed groups: combined + two bare.
    let (stdout, stderr, code) = run_rubash_file("[[ ( x && ((a ]]\n", "nested");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `]]', expected `)'",
            "line 1: expected `)'",
            "line 1: expected `)'",
            "line 1: syntax error near `]]'",
            "line 1: `[[ ( x && ((a ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_triple_paren() {
    let (stdout, stderr, code) = run_rubash_file("[[ x && (((a ]]\n", "triple");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `]]', expected `)'",
            "line 1: expected `)'",
            "line 1: expected `)'",
            "line 1: syntax error near `]]'",
            "line 1: `[[ x && (((a ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_two_line_form_reports_first_line() {
    // The closer sits on line 1: messages, near and the echoed line
    // all stay on line 1 (GNU aborts before line 2 is read as a
    // command of its own).
    let (stdout, stderr, code) = run_rubash_file("[[ x && ((a ]]\necho after\n", "twoline");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            COMBINED_ONE_BARE_NEAR_END[0],
            COMBINED_ONE_BARE_NEAR_END[1],
            COMBINED_ONE_BARE_NEAR_END[2],
            "line 1: `[[ x && ((a ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_dash_c_form() {
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ x && ((a ]]"], None);
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            COMBINED_ONE_BARE_NEAR_END[0],
            COMBINED_ONE_BARE_NEAR_END[1],
            COMBINED_ONE_BARE_NEAR_END[2],
            "line 1: `[[ x && ((a ]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_empty_group_glued_close_near() {
    // Classifier layer: the near walk crosses the glued `(' —
    // error_token_from_text reports `(]]', not `]]'.
    let (stdout, stderr, code) = run_rubash_file("[[ (]]\n", "emptyglued");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: expected `)'",
            "line 1: syntax error near `(]]'",
            "line 1: `[[ (]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_empty_group_after_and_glued_close_near() {
    let (stdout, stderr, code) = run_rubash_file("[[ x && (]]\n", "emptyand");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: expected `)'",
            "line 1: syntax error near `(]]'",
            "line 1: `[[ x && (]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_and_then_glued_close_near_includes_break_char() {
    // error_token_from_text stops ON the `&' break character and
    // includes it: near is `&]]'.
    let (stdout, stderr, code) = run_rubash_file("[[ a &&]]\n", "andend");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &["line 1: syntax error near `&]]'", "line 1: `[[ a &&]]'"],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_first_position_dparen_glued() {
    let (stdout, stderr, code) = run_rubash_file("[[ ((a]]\n", "firstpos");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: unexpected token `newline', conditional binary operator expected",
            "line 1: expected `)'",
            "line 1: expected `)'",
            "line 1: syntax error near `((a]]'",
            "line 1: `[[ ((a]]'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1554_valid_nested_groups_evaluate() {
    // Control: the closed form is a valid nested-group conditional.
    let (stdout, stderr, code) = run_rubash_file("[[ x && ((a)) ]]\necho ran=$?\n", "valid");
    assert_eq!(stdout, "ran=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
