//! Issue unixwin/rubash#1553: after a `[[ a =~ (b)]]` line (a `]]`
//! glued to the regexp group's `)', absorbed into the RHS word), the
//! tokens of the NEXT line lost their leading whitespace: the lexer's
//! PST_CONDCMD state was reset by the glued `]]` even though the word
//! — and therefore the conditional — is still open in GNU's reader,
//! so the next pass failed the leading_ws capture admission (which
//! reads the pass-start state, issue #241) and the parser's near-token
//! walk-back glued the following line's second word onto its first
//! (`echoa' for GNU's `echo'`).
//!
//! GNU owners:
//!  - parse.y:5443-5461 read_token_word + parse_matched_pair: the
//!    regexp word continues after the group closes; `]' is not a
//!    word-break character, so a depth-zero `]]` abutting the open
//!    RHS word is word text, never COND_END.
//!  - parse.y:5212: the RHS word ends at whitespace at paren depth
//!    zero — a spaced `]]` is the real closer and must still reset.
//!  - parse.y:6772-6808 error_token_from_text: the near token is the
//!    reader's current token run, which needs the true spacing.
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
    let path = std::env::temp_dir().join(format!("issue1553_{}_{}.sh", tag, std::process::id()));
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

const FILED_DIAGS: &[&str] = &[
    "line 1: syntax error in conditional expression: unexpected token `echo'",
    "line 2: syntax error near `echo'",
    "line 2: `echo after'",
];

#[test]
fn issue1553_script_form_near_keeps_next_line_spacing() {
    // The filed form: the near token is `echo', not the glued `echoa'.
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b)]]\necho after\n", "script");
    assert_eq!(stdout, "");
    expect_diags(&stderr, FILED_DIAGS);
    assert_eq!(code, Some(2));
}

#[test]
fn issue1553_dash_c_form_with_embedded_newline() {
    // The same two physical lines inside a -c string take the same
    // per-line feeder passes and must report identically.
    let (stdout, stderr, code) = run_rubash(&["-c", "[[ a =~ (b)]]\necho after"], None);
    assert_eq!(stdout, "");
    expect_diags(&stderr, FILED_DIAGS);
    assert_eq!(code, Some(2));
}

#[test]
fn issue1553_blank_line_between_keeps_line_numbers() {
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b)]]\n\necho after\n", "blank");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `echo'",
            "line 3: syntax error near `echo'",
            "line 3: `echo after'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1553_comment_line_between() {
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b)]]\n# note\necho after\n", "comment");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `echo'",
            "line 3: syntax error near `echo'",
            "line 3: `echo after'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1553_multiline_group_close_then_next_line() {
    // The group itself spans lines and its `)' lands on line 2 glued
    // to the absorbed `]]`: the state must survive that line too, and
    // the near report lands on the third physical line.
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b\n)]]\necho after\n", "multiline");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `echo'",
            "line 3: syntax error near `echo'",
            "line 3: `echo after'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1553_two_following_lines_stop_at_first() {
    // GNU's reader errors on the first token of the next line; the
    // rest of the script is never diagnosed.
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ (b)]]\necho one\necho two\n", "two");
    assert_eq!(stdout, "");
    expect_diags(
        &stderr,
        &[
            "line 1: syntax error in conditional expression: unexpected token `echo'",
            "line 2: syntax error near `echo'",
            "line 2: `echo one'",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1553_spaced_close_on_later_line_still_closes() {
    // Control: a whitespace-separated `]]` after the group is the real
    // COND_END (parse.y:5212) — the conditional evaluates normally
    // (the pattern text spans the newline, so `(b<newline>)' does not
    // match "abc" and the status is 1 — what matters is that it
    // EVALUATES instead of erroring).
    let (stdout, stderr, code) = run_rubash_file("[[ abc =~ (b\n) ]]\necho ran=$?\n", "spaced");
    assert_eq!(stdout, "ran=1\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1553_no_paren_rhs_form_unchanged() {
    // Control: without a group, the `]]` merges into the RHS word
    // token itself and the next line was already spaced correctly.
    let (stdout, stderr, code) = run_rubash_file("[[ a =~ b]]\necho after\n", "noparen");
    assert_eq!(stdout, "");
    expect_diags(&stderr, FILED_DIAGS);
    assert_eq!(code, Some(2));
}

#[test]
fn issue1553_stdin_form_reads_next_line() {
    // Activated by rubash#1573: the stdin group splitter now holds the
    // group open while the scanner's PST_CONDCMD state is armed (the
    // glued `]]` is word text, so the conditional — and the group —
    // continues onto the next line, whose first token GNU diagnoses
    // inside the conditional). FILED_DIAGS already carries the live
    // oracle's three lines, including the `` `echo after' ``
    // source-line echo, so the expectations needed no change.
    let (stdout, stderr, code) = run_rubash(&[], Some("[[ a =~ (b)]]\necho after\n"));
    assert_eq!(stdout, "");
    expect_diags(&stderr, FILED_DIAGS);
    assert_eq!(code, Some(2));
}
