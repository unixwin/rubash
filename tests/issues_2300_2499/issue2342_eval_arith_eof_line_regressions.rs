//! rubash#2342 — eval parse-error line number off by one for an
//! unclosed arithmetic expansion whose inner parens are all closed
//! (`eval 'echo $(( (2))'` reported `line 2`; GNU reports `line 1`).
//!
//! GNU mechanics (bash-5.3):
//! - The eval builtin re-parses its string WITHOUT SEVAL_RESETLINE
//!   (builtins/eval.def:55 -> parse_and_execute's `line_number--`,
//!   builtins/evalstring.c:339-346), so eval-string line N is reported
//!   at caller_line + N - 1: the string's first line shares the eval
//!   command's own line.
//! - A matched-pair EOF is reported at the line the construct OPENED:
//!   parse_matched_pair carries start_lineno (parse.y:3912) and the
//!   READ_EOF_INNER path reports it. Contiguous `$((` is the arithmetic
//!   opener in GNU's lexer, so `$(( (2))` — inner parens closed, the
//!   arithmetic group itself unclosed — is an arithmetic EOF at the
//!   open line, NOT a command-substitution EOF (which reports one past
//!   the last line). The comsub reading needs `$( (` with a space.
//!
//! rubash classified the shape in `unclosed_comsub_eof_shape`: with
//! every inner paren closed, only the `$` frame of the `$((` digraph
//! remained on the opener stack, and the arithmetic-root exemption
//! (which keys on the SECOND paren being innermost) missed, so the
//! shape fell into ComsubEof's EOF-line formula.
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! 2026-10-31).

use std::io::Write;
use std::process::Command;

/// Run `script` as a script FILE and return (stdout, stderr lines with
/// the `<path>: ` prolog stripped, rc) — same harness shape as
/// tests/issue318_eval_eof_diagnostics.rs.
fn run_script(script: &str) -> (String, Vec<String>, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2342-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).expect("create probe");
    file.write_all(script.as_bytes()).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(|line| match line.find(": eval: ") {
            Some(idx) => line[idx + 2..].to_string(),
            None => line.to_string(),
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        output.status.code(),
    )
}

const EOF_PAREN: &str = "unexpected EOF while looking for matching `)'";

fn expect_eval_error(script: &str, want_stdout: &str, want_line: usize) {
    let (stdout, stderr, rc) = run_script(script);
    assert_eq!(stdout, want_stdout, "stdout");
    assert_eq!(
        stderr,
        vec![format!("eval: line {want_line}: {EOF_PAREN}")],
        "stderr"
    );
    // The scripts end in `echo rc=$?`, so the script's own exit code is
    // the echo's 0 — which is itself the proof that the script continued
    // past the failed eval (whose status 2 is in the echoed text).
    assert_eq!(rc, Some(0), "script rc");
}

/// The issue seed: the whole string fails to parse (nothing runs),
/// eval returns 2, and the error names the eval command's own line.
#[test]
fn seed_reports_open_line() {
    expect_eval_error("eval 'echo $(( (2))'\necho rc=$?\n", "rc=2\n", 1);
}

/// An eval on script line 2 reports line 2, not 3.
#[test]
fn eval_on_second_line_reports_second_line() {
    expect_eval_error(
        "echo start\neval 'echo $(( (2))'\necho rc=$?\n",
        "start\nrc=2\n",
        2,
    );
}

/// The construct on eval-string line 2 (after an executed prefix line)
/// reports caller_line + 1.
#[test]
fn construct_on_string_line_two_reports_caller_plus_one() {
    expect_eval_error(
        "eval 'echo ok\necho $(( (2))'\necho rc=$?\n",
        "ok\nrc=2\n",
        2,
    );
}

/// Inside a function, GNU line numbers are function-relative — but
/// the frame's first counted line is the `{` line, so the eval on the
/// body's first statement line reports line 2, not 1. Oracle-checked
/// against GNU bash 5.3.20 with this exact script (stderr: "eval:
/// line 2: unexpected EOF while looking for matching `)'"); the
/// original expectation of 1 was a wrong pin.
#[test]
fn function_body_eval_reports_function_relative_line() {
    expect_eval_error(
        "f() {\n  eval 'echo $(( (2))'\n}\nf\necho rc=$?\n",
        "rc=2\n",
        2,
    );
}

/// A two-line string whose construct is on line 1 reports line 1 —
/// the report follows the OPEN line, not the end of the string.
#[test]
fn open_line_wins_over_string_length() {
    expect_eval_error(
        "eval 'echo $(( (2))\necho after'\necho rc=$?\n",
        "rc=2\n",
        1,
    );
}

/// The arithmetic group nested inside an also-unclosed command
/// substitution still reports the arithmetic open line (GNU-verified).
#[test]
fn arith_nested_in_unclosed_comsub_reports_arith_line() {
    expect_eval_error("eval 'echo $(( (echo'\necho rc=$?\n", "rc=2\n", 1);
}

/// Trailing text after the closed inner parens changes nothing.
#[test]
fn trailing_text_keeps_open_line() {
    expect_eval_error("eval 'echo $(( (2)) x'\necho rc=$?\n", "rc=2\n", 1);
}

/// `$((echo hi)` — the digraph's second paren is the subshell-shaped
/// frame and it closed; the arithmetic root is what remains unclosed.
#[test]
fn closed_inner_group_leaves_arith_root() {
    expect_eval_error("eval 'echo $((echo hi)'\necho rc=$?\n", "rc=2\n", 1);
}

/// `$((a)` — same root, minimal body.
#[test]
fn minimal_unclosed_arith_reports_open_line() {
    expect_eval_error("eval 'echo $((a)'\necho rc=$?\n", "rc=2\n", 1);
}

/// Control: a genuine unclosed command substitution still reports the
/// EOF line (one past the string's last line).
#[test]
fn comsub_eof_line_unchanged() {
    expect_eval_error("eval 'echo $(foo'\necho rc=$?\n", "rc=2\n", 2);
}

/// Control: the spaced `$( (` form IS a comsub containing a subshell —
/// the EOF line, unchanged by the arithmetic-root rule.
#[test]
fn spaced_comsub_subshell_keeps_eof_line() {
    expect_eval_error("eval 'echo $( (2'\necho rc=$?\n", "rc=2\n", 2);
}

/// Control: an unclosed quote still reports its open line.
#[test]
fn quote_eof_line_unchanged() {
    let (stdout, stderr, rc) = run_script("eval \"echo 'abc\"\necho rc=$?\n");
    assert_eq!(stdout, "rc=2\n", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 1: unexpected EOF while looking for matching `''".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(0));
}
