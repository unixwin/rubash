//! Regression pins for rubash#3664 — double quotes inside a backquote
//! command substitution inside double quotes caused a syntax error on
//! the eval re-parse path.
//!
//! The mechanism, from GNU source: inside a double-quoted word, a
//! backquote opens a command substitution whose body is a unit —
//! read_token_word hands the quote character to parse_matched_pair
//! (parse.y:5419-5440) and string_extract_double_quoted runs in
//! backquote mode (subst.c:925-932), so a `"` inside the body neither
//! closes the enclosing quote nor toggles quote state; only an
//! unescaped backtick ends the unit. rubash's unclosed-comsub shape
//! scan (src/lexer/skip.rs `unclosed_comsub_eof_shape`) tracked the
//! double-quote state byte by byte without skipping the backquote
//! unit, so the body's first `"` closed the quote in the scan, the
//! quoted `(` landed on its paren stack as a bare opener, and the
//! eval re-parse route fabricated `syntax error near unexpected
//! token `('` for a well-formed string. The direct (non-eval) parse
//! was never affected — its lexer already treats the unit whole.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts (oracle-verified by the fix lane, 2026-11-15).
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3664-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::canonicalize(&dir).expect("canonical root")
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn expect(root: &Path, body: &str, rc: i32, stdout: &str, stderr: &str) {
    let out = run(root, body);
    assert_eq!(out.rc, rc, "body: {body:?}");
    assert_eq!(out.stdout, stdout, "body: {body:?}");
    assert_eq!(out.stderr, stderr, "body: {body:?}");
}

/// The issue seed: byte-exact line 10 of modernish
/// cap/BUG_CSUBBTQUOT.t. GNU's eval succeeds, so `!` flips its 0 to 1;
/// the buggy scan made the subshell die on a fabricated token error,
/// whose non-zero status `!` flipped to 0.
#[test]
fn seed_line10_rc_is_1() {
    let root = scratch("seed");
    expect(
        &root,
        "! (eval '_Msh_test=\"`: \"(\"`\"') 2>/dev/null\necho \"rc=$?\"\n",
        0,
        "rc=1\n",
        "",
    );
}

/// The eval assignment form: the comsub body is `: "("` — a quoted
/// paren inside the backquote unit inside the double quotes.
#[test]
fn eval_assignment_backquote_dquote_paren() {
    let root = scratch("assign");
    expect(
        &root,
        "eval '_Msh_test=\"`: \"(\"`\"'\necho \"rc=$? val=<$_Msh_test>\"\n",
        0,
        "rc=0 val=<>\n",
        "",
    );
}

/// The same unit in an echo word through eval.
#[test]
fn eval_echo_backquote_dquote_paren() {
    let root = scratch("echo");
    expect(
        &root,
        "eval 'echo \"a`: \"(\"`b\"'\necho \"rc=$?\"\n",
        0,
        "ab\nrc=0\n",
        "",
    );
}

/// Literal text around the unit inside the enclosing double quotes.
#[test]
fn eval_assignment_with_surrounding_text() {
    let root = scratch("surround");
    expect(
        &root,
        "eval '_Msh_test=\"PRE`: \"(\"`POST\"; echo \"val=<$_Msh_test>\"'\n",
        0,
        "val=<PREPOST>\n",
        "",
    );
}

/// A quoted paren later in the body, after other quoted text.
#[test]
fn eval_body_quoted_paren_after_other_text() {
    let root = scratch("later");
    expect(
        &root,
        "eval 'echo \"a`: \"x(y\"`b\"'\necho \"rc=$?\"\n",
        0,
        "ab\nrc=0\n",
        "",
    );
}

/// Control: the direct (non-eval) parse of the same assignment was
/// always correct; the pin keeps the two paths locked together.
#[test]
fn direct_assignment_control() {
    let root = scratch("direct");
    expect(
        &root,
        "x=\"`: \"(\"`\"\necho \"rc=$? x=<$x>\"\n",
        0,
        "rc=0 x=<>\n",
        "",
    );
}

/// Control: a backquote body whose inner double quotes wrap no
/// metacharacter — this passed even with the buggy scan (the toggled
/// quote state happened to re-close), and must stay green.
#[test]
fn eval_backquote_dquotes_no_paren_control() {
    let root = scratch("noparen");
    expect(
        &root,
        "eval 'echo \"a`echo \"x\"`b\"'\necho \"rc=$?\"\n",
        0,
        "axb\nrc=0\n",
        "",
    );
}

/// Control: the $(...) form of the same shape through eval.
#[test]
fn eval_dollar_paren_control() {
    let root = scratch("dolparen");
    expect(
        &root,
        "eval 'echo \"a$(echo \"x\")b\"'\necho \"rc=$?\"\n",
        0,
        "axb\nrc=0\n",
        "",
    );
}

/// Negative control: a genuine token error inside the backquote body
/// is still reported by the body's own parse, byte-identical to GNU —
/// the fix must not blind the scan to real errors.
#[test]
fn genuine_comsub_body_error_still_reported() {
    let root = scratch("genuine");
    expect(
        &root,
        "eval 'echo \"a`echo (`b\"'\necho \"rc=$?\"\n",
        0,
        "ab\nrc=0\n",
        "case.sh: command substitution: line 1: syntax error near unexpected token `newline'\ncase.sh: command substitution: line 1: `echo ('\n",
    );
}

/// Negative control: a genuinely unterminated double quote (the
/// backquote unit IS closed; the quote is not) still takes the
/// matching-quote EOF report with exit 2.
#[test]
fn unterminated_dquote_still_eof_error() {
    let root = scratch("unterminated");
    expect(
        &root,
        "eval 'echo \"a`: \"(\"`'\necho \"rc=$?\"\n",
        0,
        "rc=2\n",
        "case.sh: eval: line 1: unexpected EOF while looking for matching `\"'\n",
    );
}
