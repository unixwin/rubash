//! rubash#3388 — an unclosed `[` holding a quoted paren pair used to kill
//! the whole script at parse time.
//!
//! Root cause: the function-definition probe in
//! `src/parser/function_command.rs` matched the `()` head by token VALUE
//! only, so the quoted words `'('` / `')'` / `'()'` (ordinary Word tokens
//! whose dequoted value is `(` / `)` / `()`) were mistaken for the
//! `WORD '(' ')'` function head. The probe then demanded a `{` body and
//! reported a syntax error at whatever followed (`;`, `]`, the next word,
//! or EOF), aborting the rest of the input — GNU treats `[` as an ordinary
//! builtin, so a missing `]` is only a runtime `[: missing ']'` error
//! (status 2) and the script continues.
//!
//! GNU baseline: bash 5.2.21 on this box (the issue was filed against
//! 5.3.20; the behaviour is identical). Every expectation below was
//! verified against GNU first; diagnostics differ only in the program-name
//! prefix (`bash` vs the rubash binary path), so stderr is asserted with
//! `contains` while stdout and the exit status are exact.

use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_script(body: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue3388-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue3388 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, body).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .output()
        .expect("run rubash probe");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn run_dash_c(script: &str) -> RunOutcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .output()
        .expect("run rubash -c probe");
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The issue's regression seed, verbatim: GNU scores 4/4 PASS.
#[test]
fn issue_seed_scores_four_of_four() {
    let outcome = run_script(
        r#"[ '(' a = a ')' ]
[ $? -eq 0 ] && echo "balanced-paren: PASS" || echo "balanced-paren: FAIL"
[ a = a ]
[ $? -eq 0 ] && echo "plain-bracket: PASS" || echo "plain-bracket: FAIL"
[ '(' ')'
rc=$?
[ "$rc" -eq 2 ] && echo "missing-bracket-rc2: PASS" || echo "missing-bracket-rc2: FAIL (got $rc)"
echo "survived-missing-bracket: PASS"
"#,
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(
        outcome.stdout,
        "balanced-paren: PASS\nplain-bracket: PASS\nmissing-bracket-rc2: PASS\nsurvived-missing-bracket: PASS\n"
    );
    assert!(
        outcome.stderr.contains("[: missing `]'"),
        "stderr: {}",
        outcome.stderr
    );
    assert!(
        !outcome.stderr.contains("syntax error"),
        "stderr: {}",
        outcome.stderr
    );
}

/// The `-c` form from the issue: the `[` reports rc 2, the `echo` runs.
#[test]
fn dash_c_form_reports_rc2_and_continues() {
    let outcome = run_dash_c("[ '(' ')'; echo rc=$?");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "rc=2\n");
    assert!(
        outcome.stderr.contains("[: missing `]'"),
        "stderr: {}",
        outcome.stderr
    );
    assert!(
        !outcome.stderr.contains("syntax error"),
        "stderr: {}",
        outcome.stderr
    );
}

/// Quoted parens are plain words for any command, not just `[` — the same
/// probe used to eat them as a function head (`echo '(' ')'` died at `;`).
#[test]
fn quoted_parens_are_plain_words_for_any_command() {
    let outcome = run_dash_c("echo '(' ')' ; echo hi");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "( )\nhi\n");
    let outcome = run_dash_c("echo '()'; echo hi");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "()\nhi\n");
    let outcome = run_dash_c("foo '(' ')'; echo hi");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "hi\n");
    assert!(
        outcome.stderr.contains("foo: command not found"),
        "stderr: {}",
        outcome.stderr
    );
}

/// A CLOSED `[` with quoted parens is a runtime test error, not a parse
/// error — and the script continues past it.
#[test]
fn closed_bracket_with_quoted_parens_is_a_runtime_error() {
    let outcome = run_dash_c("[ '(' ')' ]; echo hi");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "hi\n");
    assert!(
        outcome.stderr.contains("[: (: unary operator expected"),
        "stderr: {}",
        outcome.stderr
    );
    let outcome = run_dash_c("[ '(' ')' a ]; echo hi");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "hi\n");
    assert!(
        outcome.stderr.contains("[: ): binary operator expected"),
        "stderr: {}",
        outcome.stderr
    );
}

/// `!` and `test` neighbours behave the same as GNU.
#[test]
fn bang_and_test_forms_match_gnu() {
    let outcome = run_dash_c("! [ '(' ')'; echo hi");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "hi\n");
    assert!(
        outcome.stderr.contains("[: missing `]'"),
        "stderr: {}",
        outcome.stderr
    );
    let outcome = run_dash_c("test '(' ')'; echo hi");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "hi\n");
    assert!(
        outcome.stderr.contains("test: (: unary operator expected"),
        "stderr: {}",
        outcome.stderr
    );
}

/// UNQUOTED parens keep their grammar meaning: these are still parse
/// errors in both shells, at the same token GNU blames.
#[test]
fn unquoted_parens_still_parse_errors() {
    let outcome = run_dash_c("[ ( a = a ); echo hi");
    assert_eq!(outcome.code, Some(2));
    assert!(outcome.stdout.is_empty());
    assert!(
        outcome
            .stderr
            .contains("syntax error near unexpected token `a'"),
        "stderr: {}",
        outcome.stderr
    );
    let outcome = run_dash_c("[ a ( b ]; echo hi");
    assert_eq!(outcome.code, Some(2));
    assert!(
        outcome
            .stderr
            .contains("syntax error near unexpected token `('"),
        "stderr: {}",
        outcome.stderr
    );
    let outcome = run_dash_c("echo (); echo hi");
    assert_eq!(outcome.code, Some(2));
    assert!(
        outcome
            .stderr
            .contains("syntax error near unexpected token `;'"),
        "stderr: {}",
        outcome.stderr
    );
    let outcome = run_dash_c("[ ( ) ]; echo hi");
    assert_eq!(outcome.code, Some(2));
    assert!(
        outcome
            .stderr
            .contains("syntax error near unexpected token `]'"),
        "stderr: {}",
        outcome.stderr
    );
}

/// `[[ ]]` is out of scope for this fix: an unclosed `[[` is a parse
/// error in both shells and must stay one.
#[test]
fn double_bracket_unclosed_still_parse_error() {
    let outcome = run_dash_c("[[ a = a; echo hi");
    assert_eq!(outcome.code, Some(2));
    assert!(outcome.stdout.is_empty());
    assert!(
        outcome
            .stderr
            .contains("syntax error in conditional expression"),
        "stderr: {}",
        outcome.stderr
    );
}

/// Genuine function definitions (all three spellings) still define and run.
#[test]
fn function_definitions_still_work() {
    let outcome = run_dash_c("foo () { echo INFUNC; }; foo");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "INFUNC\n");
    let outcome = run_dash_c("f() { echo hi; }; f");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "hi\n");
    let outcome = run_dash_c("function f() { echo hi; }; f");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "hi\n");
}
