//! Regressions for unixwin/rubash#462: a function-definition name word
//! containing an unquoted `$( ... )` was rejected by the parser with
//! `syntax error near unexpected token '('` — GNU's function_def grammar
//! accepts ANY single WORD before `()`, and the lexer folds an unquoted
//! command substitution into the current word (parse.y:5513-5532 `$('
//! branch), so `f$()g' is one WORD and `f$()g() { :; }' parses. The name is
//! validated at DEFINITION time (execute_cmd.c execute_intern_function ->
//! general.c valid_function_word): a name containing `$' is rejected
//! non-fatally with `` `NAME': not a valid identifier'` (rc=1) and the
//! script continues. This broke zZshFramework files.sh
//! (`fileRemoveWithOverwrite-dirOrFile$(useWithCaution)__hsl() { ... }`).
//!
//! Every expectation below was measured against the GNU oracle
//! (`D:\Git\usr\bin\bash.exe`, GNU bash 5.2.37 msys) on 2026-10-10; the
//! issue's own table was taken against WSL GNU Bash 5.3.0.

use std::process::Command;

fn run_rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// GNU parses the definition, rejects the expanded/raw name at definition
/// time with `` `RAW': not a valid identifier'` (rc=1, non-fatal), and
/// continues with the rest of the script. `raw_fragment` is a quote-free
/// piece of the raw name (the diagnostic renderer re-renders embedded
/// quotes), matched verbatim inside the `` `...': not a valid identifier'`
/// message.
fn assert_gnu_definition_rejected(script: &str, raw_fragment: &str, later_stdout: &str) {
    let (stdout, stderr, code) = run_rubash(script);
    assert_eq!(
        code,
        Some(0),
        "rc mismatch for {script:?} (stderr={stderr:?})"
    );
    assert_eq!(stdout, later_stdout, "stdout mismatch for {script:?}");
    let stderr = stderr.replace("\"", "'");
    let marker = "not a valid identifier";
    let message = stderr
        .lines()
        .find(|line| line.contains(marker))
        .unwrap_or_else(|| panic!("missing `{marker}' diagnostic: {stderr:?}"));
    assert!(
        message.contains(raw_fragment),
        "definition-time name error must echo the raw name {raw_fragment:?}: {stderr:?}"
    );
    // -n (noexec) parses the same input cleanly: the issue's own repro is a
    // parse-acceptance failure, so the no-exec pass must be silent rc=0.
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-n", "-c", script])
        .output()
        .expect("run rubash -n");
    assert!(
        output.status.success(),
        "-n must accept {script:?}: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ---------------------------------------------------------------------------
// The issue's minimal repro and boundary table
// ---------------------------------------------------------------------------

#[test]
fn issue_repro_unquoted_cmdsub_in_name() {
    assert_gnu_definition_rejected("f$()g() { :; }\necho ok\n", "f$()g", "ok\n");
}

#[test]
fn unquoted_cmdsub_mid_name() {
    // zZshFramework files.sh line 5 shape.
    assert_gnu_definition_rejected(
        "fileRemoveWithOverwrite-dirOrFile$(useWithCaution)__hsl() { :; }\necho ok\n",
        "fileRemoveWithOverwrite-dirOrFile$(useWithCaution)__hsl",
        "ok\n",
    );
}

#[test]
fn quoted_cmdsub_in_name_still_parses() {
    // Was already accepted (raw != value -> quoted path); the fix must not
    // change it: GNU reports the same definition-time identifier error.
    assert_gnu_definition_rejected("f\"$(echo x)\"g() { :; }\necho ok\n", "$(echo x)", "ok\n");
}

#[test]
fn bare_expansion_name() {
    // `$x() { :; }` lexes the name as a Variable token — the parse-loop
    // gate must admit the token kind, not only plain words.
    assert_gnu_definition_rejected("$x() { :; }\necho ok\n", "$x", "ok\n");
}

#[test]
fn cmdsub_only_name() {
    assert_gnu_definition_rejected("$(echo f)() { :; }\necho ok\n", "$(echo f)", "ok\n");
}

#[test]
fn simple_expansion_in_name_unchanged() {
    assert_gnu_definition_rejected("f$xeeg() { :; }\necho ok\n", "f$xeeg", "ok\n");
}

#[test]
fn plain_name_control_row() {
    // The control row: a plain name DEFINES the function (GNU: no error).
    let (stdout, stderr, code) = run_rubash("name() { echo hi; }\nname\n");
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "hi\n");
    assert!(stderr.is_empty(), "stderr={stderr:?}");
}

// ---------------------------------------------------------------------------
// Keyword form and backtick form (GNU 5.2 oracle, same acceptance)
// ---------------------------------------------------------------------------

#[test]
fn keyword_form_cmdsub_name() {
    assert_gnu_definition_rejected("function f$()g { :; }\necho ok\n", "f$()g", "ok\n");
}

#[test]
fn keyword_form_cmdsub_name_with_parens() {
    assert_gnu_definition_rejected(
        "function f$(echo x)g() { :; }\necho ok\n",
        "f$(echo x)g",
        "ok\n",
    );
}

#[test]
fn backtick_cmdsub_only_name() {
    assert_gnu_definition_rejected("`echo f`() { :; }\necho ok\n", "`echo f`", "ok\n");
}

// ---------------------------------------------------------------------------
// Break characters OUTSIDE the substitution span still end the word — the
// masked admission only sees span interiors. `f$();g() { :; }' is NOT a
// function definition anywhere: the word `f$()' is a command, `;' separates,
// and `g()' defines g (GNU oracle: `f: command not found', `declare -f g'
// lists g, rc=0).
// ---------------------------------------------------------------------------

#[test]
fn semicolon_outside_span_still_ends_the_word() {
    let (stdout, stderr, code) = run_rubash("f$();g() { echo G; }\necho A\ng\n");
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "A\nG\n", "g must be defined and callable");
    assert!(
        stderr.contains("f: command not found"),
        "the head word runs as a command: {stderr:?}"
    );
}

#[test]
fn pipe_outside_span_still_ends_the_word() {
    let (stdout, stderr, code) = run_rubash("f$()|g() { :; }\necho B\n");
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "B\n");
    assert!(
        stderr.contains("f: command not found"),
        "the head word runs as a command: {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// The name admission must not swallow the body's own syntax errors: GNU
// reports a bad body as `syntax error near unexpected token `)''.
// ---------------------------------------------------------------------------

#[test]
fn bad_body_still_reports_parse_error() {
    let (_, stderr, code) = run_rubash("f$()g() { echo ); }\n");
    assert_eq!(code, Some(2));
    assert!(
        stderr.contains("syntax error near unexpected token `)'"),
        "body error must surface: {stderr:?}"
    );
}

#[test]
fn assignment_prefix_still_binds_before_cmdsub_name_gate() {
    // #453 family: an assignment-prefix word must stay an assignment, not a
    // function definition — `a=$(echo b)` followed by `()` never appears in
    // GNU's function_def either (the word holds `=').
    let (stdout, _stderr, code) = run_rubash("a=$(echo b)\necho \"$a\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "b\n");
}
