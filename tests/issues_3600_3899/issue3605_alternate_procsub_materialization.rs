//! Issue rubash#3605 (run-new-exp): an unquoted `<(`/`>(` inside the
//! `word` half of `${var op word}` must materialize when the arm that
//! expands the word is taken, in every position the operator word can
//! appear — a whole-word argument, an embedded `${}` fragment, and an
//! assignment's bound value. GNU expands the word through
//! parameter_brace_expand_word (subst.c:7663), which hands the text back
//! to the same expand_word_internal scan that owns the `<`/`>`
//! process-substitution cases (subst.c:11365-11385), so
//! `cat ${foo:-<(echo a)}` reads /dev/fd/N.
//!
//! The same GNU scan also fixes the boundary of the feature: a `<(` or
//! `>(` inside a double-quoted span is skipped (subst.c:11369
//! Q_DOUBLE_QUOTES), and text arriving from a variable's VALUE is data —
//! expansion output is appended, never re-scanned — so
//! `x='<(echo V)'; echo ${x:-<(echo W)}` prints the stored `<(echo V)`.
//!
//! Expected outputs verified byte-for-byte against GNU bash 5.3.20
//! (script files).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
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

/// Whole-word `:-` alternate on an unset variable: `cat` reads the
/// materialized fd.
#[test]
fn colon_default_argument_materializes() {
    let (stdout, _stderr, code) = rubash("unset u; cat ${u:-<(echo A)}");
    assert_eq!(stdout.trim_end(), "A");
    assert_eq!(code, Some(0));
}

/// A taken `:-` uses the VALUE when the variable is set — the alternate
/// is never expanded, so no process substitution runs.
#[test]
fn colon_default_uses_value_when_set() {
    let (stdout, _stderr, code) = rubash("foo=a; echo ${foo:-<(echo E)}");
    assert_eq!(stdout.trim_end(), "a");
    assert_eq!(code, Some(0));
}

/// Whole-word `+` alternate on a set variable (the `braced_alternate_word_values`
/// fast path used to render it literal).
#[test]
fn plus_alternate_argument_materializes() {
    let (stdout, _stderr, code) = rubash("foo=a; cat ${foo+<(echo G)}");
    assert_eq!(stdout.trim_end(), "G");
    assert_eq!(code, Some(0));
}

/// `:=` binds the materialized /dev/fd path into the variable.
#[test]
fn colon_assign_binds_fd_path() {
    let (stdout, _stderr, code) = rubash("unset u; echo ${u:=<(echo H)}");
    assert!(stdout.trim_end().starts_with("/dev/fd/"), "stdout={stdout}");
    assert_eq!(code, Some(0));
}

/// Embedded `${foo+...}` inside a larger word — needs the mutable walk
/// because materializing a process substitution spawns a child.
#[test]
fn embedded_plus_alternate_materializes() {
    let (stdout, _stderr, code) = rubash("foo=a; echo 9:${foo+<(echo G)}");
    assert!(
        stdout.trim_end().starts_with("9:/dev/fd/"),
        "stdout={stdout}"
    );
    assert_eq!(code, Some(0));
}

/// Value-borne `<(` stays data: expansion results are never re-scanned.
#[test]
fn value_borne_procsub_text_stays_literal() {
    let (stdout, _stderr, code) = rubash("x='<(echo V)'; echo ${x:-<(echo W)}");
    assert_eq!(stdout.trim_end(), "<(echo V)");
    assert_eq!(code, Some(0));
}

/// A `<(` inside a double-quoted word is skipped (Q_DOUBLE_QUOTES).
#[test]
fn double_quoted_alternate_stays_literal() {
    let (stdout, _stderr, code) = rubash("unset u; echo \"${u:-<(echo D)}\"");
    assert_eq!(stdout.trim_end(), "<(echo D)");
    assert_eq!(code, Some(0));
}

/// Assignment-RHS form: the bound value keeps value-borne `<(` literal.
#[test]
fn assignment_rhs_value_borne_stays_literal() {
    let (stdout, _stderr, code) = rubash("x='<(echo a)'; y=${x:-<(echo b)}; printf '<%s>' \"$y\"");
    assert_eq!(stdout.trim_end(), "<<(echo a)>");
    assert_eq!(code, Some(0));
}
