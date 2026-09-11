//! POSIX Interp 221 regression: under `set -o posix`, the dolbrace pairing of
//! a `${...}` body depends on each word's own quote state — a `'` inside a
//! double-quoted `${...}` is literal data (the first `}` closes), while
//! unquoted it opens a nested single quote. The single-command command
//! substitution shortcuts split the body with a generic quote scanner that
//! strips quotes inside `${...}` bodies before expansion
//! (`echo ${IFS+'}'z}` arrived at the echo shortcut as the corrupted word
//! `${IFS+}z}`), so posixexp2.tests cases 11/12 mis-paired.
//! GNU baseline: WSL GNU Bash 5.2.21, script-file comparison.

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

#[test]
fn posix_dquoted_brace_body_first_brace_closes_inside_comsub() {
    // posixexp2.tests case 11: the inner word is double-quoted, so the `'`
    // characters are literal data and the first `}` closes the expansion.
    let (stdout, stderr, code) = rubash(r#"set -o posix; echo "$(echo "${IFS+'}'z}")""#);
    assert_eq!(stdout, "''z}\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn posix_unquoted_brace_body_nested_quote_opens_inside_comsub() {
    // posixexp2.tests case 12: the inner word is unquoted, so `'...'` opens a
    // nested quoted string and the second `}` closes the expansion; the
    // alternate `'}'` quote-removes to `}`.
    let (stdout, stderr, code) = rubash(r#"set -o posix; echo "$(echo ${IFS+'}'z})""#);
    assert_eq!(stdout, "}z\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn posix_dquoted_brace_body_plain_word_unchanged() {
    // posixexp2.tests case 2: already correct before the fix; guards the
    // plain (non-command-substitution) path against collateral changes.
    let (stdout, stderr, code) = rubash(r#"set -o posix; echo "${IFS+'}'z}""#);
    assert_eq!(stdout, "''z}\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn non_posix_brace_body_keeps_quote_operator_pairing() {
    // Without `set -o posix` the single-quote operator pairing applies in
    // both quote contexts, and the fast paths stay on their shortcuts.
    // GNU 5.2.21: the dquoted word yields `'}'z` (the quote characters are
    // literal data in the output), the unquoted word yields `}z`.
    let (stdout, _stderr, code) = rubash(r#"echo "${IFS+'}'z}""#);
    assert_eq!(stdout, "'}'z\n");
    assert_eq!(code, Some(0));

    let (stdout, _stderr, code) = rubash(r#"echo "$(echo ${IFS+'}'z})""#);
    assert_eq!(stdout, "}z\n");
    assert_eq!(code, Some(0));
}

#[test]
fn assignment_alternate_unquoted_is_quote_removed_and_field_split() {
    // posixexp2.tests case 35: the unquoted `=` alternate is quote-removed
    // before assignment, and the expansion result field-splits.
    let (stdout, _stderr, code) =
        rubash(r#"set -o posix; unset v; printf '<%s> ' ${v=a\ b} x ${v=c\ d}"#);
    assert_eq!(stdout, "<a> <b> <x> <a> <b> ");
    assert_eq!(code, Some(0));
}

#[test]
fn assignment_alternate_double_quoted_keeps_literal_backslash() {
    // posixexp2.tests case 36: inside double quotes `\` escapes only
    // $, `, ", \, and newline, so `\ ` stays literal in the assigned value
    // and the quoted expansion prints it verbatim.
    let (stdout, _stderr, code) =
        rubash(r#"set -o posix; unset v; printf '<%s> ' "${v=a\ b}" x "${v=c\ d}""#);
    assert_eq!(stdout, "<a\\ b> <x> <a\\ b> ");
    assert_eq!(code, Some(0));
}

#[test]
fn word_alternate_unquoted_never_field_splits() {
    // posixexp2.tests case 37: the `-` alternate undergoes quote removal but
    // its quoted-space markers survive, so the result stays one field.
    let (stdout, _stderr, code) =
        rubash(r#"set -o posix; unset v; printf '<%s> ' ${v-a\ b} x ${v-c\ d}"#);
    assert_eq!(stdout, "<a b> <x> <c d> ");
    assert_eq!(code, Some(0));
}

#[test]
fn assignment_inside_command_substitution_uses_inner_quote_context() {
    // The `${v=...}` body inside a command substitution does not inherit the
    // enclosing double-quoted word's quote context: GNU assigns `a b` here.
    let (stdout, _stderr, code) = rubash(r#"unset v; echo "$(printf '<%s> ' ${v=a\ b})""#);
    assert_eq!(stdout, "<a> <b> \n");
    assert_eq!(code, Some(0));
}

#[test]
fn escaped_quote_in_unquoted_alternate_yields_literal_quote() {
    // posixexp2.tests cases 8/14: `\"` inside a `${...}` body is a literal
    // data quote, not a syntax quote the expansion stages can swallow.
    let (stdout, _stderr, code) = rubash(r#"set -o posix; unset v; echo "${IFS+\"}""#);
    assert_eq!(stdout, "\"\n");
    assert_eq!(code, Some(0));
}

#[test]
fn posix_interleaved_quotes_case28_mixed_quoted_word() {
    // posixexp2.tests case 28: the word is a dq segment + sq segment pair.
    // POSIX closes the `${...}` at the first `}` (single quotes literal in
    // the double-quoted body), the `"` after `'x` closes the dq segment, and
    // `'...'` single-quotes the tail. GNU 5.2.21:
    // `'x ~ x''x}"x}" #`.
    let (stdout, stderr, code) = rubash(
        "set -o posix ; shopt -u xpg_echo\n(echo -n '28 '; printf '%s\n' \"${IFS+\"'\"x ~ x'}'x\"'}\"x}\" #') 2>&- || echo failed in 28\n",
    );
    assert_eq!(stdout, "28 'x ~ x''x}\"x}\" #\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn posix_interleaved_quotes_case28_dq_segment_alone() {
    // The dq-segment-only sub-case: GNU 5.2.21 prints `'x ~ x'`.
    let (stdout, stderr, code) = rubash("set -o posix\nprintf '<%s>\n' \"${IFS+\"'\"x ~ x'}\"\n");
    assert_eq!(stdout, "<'x ~ x'>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn posix_interleaved_quotes_case28_dq_segment_plus_sq_tail() {
    // dq segment followed by an unterminated-looking `'x"` tail: GNU 5.2.21
    // prints `'x ~ x''x` (the trailing `"` closes the double quote).
    let (stdout, stderr, code) = rubash("set -o posix\nprintf '<%s>\n' \"${IFS+\"'\"x ~ x'}'x\"\n");
    assert_eq!(stdout, "<'x ~ x''x>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
