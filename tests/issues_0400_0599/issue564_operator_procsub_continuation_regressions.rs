//! Issue rubash#564: an OPERATOR-position process substitution whose body
//! spans physical lines — `cat <(` NEWLINE ... NEWLINE `)` — was rejected
//! immediately (`syntax error near unexpected token `('`) while GNU keeps
//! reading. GNU read_token's process-substitution check (parse.y:3776-3796)
//! declines to return the `<`/`>` operator when peek_char == '(' and falls
//! through to read_token_word (`tokword:`), whose shellexp arm
//! (parse.y:5494-5524) consumes the `(...)` body via parse_comsub — a
//! command-context reader whose matched-pair scan treats newlines as
//! ordinary body characters, so read_secondary_line keeps pulling physical
//! lines until the matching `)`. The word-ATTACHED form (`word<(`) is
//! rubash#463; this issue is the space-separated form the #463 boundary
//! table deliberately left as a "pre-existing `-c`/script gap".
//!
//! Oracle: WSL GNU bash 5.3.0(1)-release, `bash -s` on every shape below
//! (2026-10-12 probes): `echo <(echo` NL `hi` NL `)` executes with stdout
//! `/dev/fd/63` (the body's `hi` goes to the pipe), `cat <(echo` NL
//! `echo hi2` NL `)` runs BOTH body lines (hi2 on stdout), an unterminated
//! body reports "unexpected EOF while looking for matching `)'" at the EOF
//! line and exits 2.

use std::process::{Command, Stdio};

fn rubash_args(script: &str, args: &[&str]) -> (String, String, Option<i32>) {
    let file = std::env::temp_dir().join(format!("issue564-{}.sh", std::process::id()));
    std::fs::write(&file, script).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .arg(&file)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&file);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    rubash_args(script, &[])
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -s");
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script");
    let output = child.wait_with_output().expect("wait rubash -s");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's shape: the reader must join `echo <(echo` with the following
/// lines until the `)` closes the substitution, then keep parsing (`echo
/// after` runs). GNU stdout: `/dev/fd/63` + `after` (the body's `hi` is
/// written to the pipe nobody drains).
#[test]
fn minimal_repro_executes_across_lines() {
    let script = "echo <(echo\nhi\n)\necho after\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "rc={code:?} stderr={stderr}");
    assert_eq!(stdout, "/dev/fd/63\nafter\n", "GNU-identical stdout");
    assert!(
        !stderr.contains("syntax error"),
        "parse must not fail: {stderr}"
    );
}

/// Same shape through the stdin `-s` reader (the issue's own reproduction
/// channel).
#[test]
fn stdin_script_keeps_reading() {
    let script = "echo <(echo\nhi\n)\necho after\n";
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "rc={code:?} stderr={stderr}");
    assert_eq!(stdout, "/dev/fd/63\nafter\n", "GNU-identical stdout");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
}

/// GNU cross-line BODY semantics: every body line executes inside the
/// substitution (`cat <(echo` NL `echo hi2` NL `)` runs `echo` into the
/// pipe — the empty line cat drains — and `echo hi2` on the inherited
/// stdout). Locks that the joined body is re-parsed as command text, not
/// flattened into one word.
#[test]
fn body_lines_execute_inside_substitution() {
    let script = "cat <(echo\necho hi2\n)\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "rc={code:?} stderr={stderr}");
    assert!(
        stdout.contains("hi2"),
        "body's second line must run: stdout={stdout:?}"
    );
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
}

/// `>(` spans lines the same way (parse.y:3792 checks both operators).
#[test]
fn output_procsub_spans_lines() {
    let script = "echo hi > >(cat\n)\necho done\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "rc={code:?} stderr={stderr}");
    assert!(stdout.contains("hi"), "stdout: {stdout:?}");
    assert!(stdout.contains("done"), "stdout: {stdout:?}");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
}

/// EOF inside the body takes the yyerror route: "unexpected EOF while
/// looking for matching `)'" at the EOF line (input `echo <(echo` + NL
/// reports line 2), exit status 2 — GNU-verified on 5.3.0.
#[test]
fn unclosed_body_eof_diagnostic() {
    let script = "echo <(echo\n";
    let (_, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(2), "GNU exits 2: {stderr}");
    assert!(
        stderr.contains("line 2: unexpected EOF while looking for matching `)'"),
        "stderr: {stderr}"
    );
}

/// The reader must not swallow the following command: a CLOSED substitution
/// ends the logical line and later lines parse standalone (GNU: `after` on
/// its own line, and `done` must NOT be captured into the substitution).
#[test]
fn closed_body_releases_the_reader() {
    let script = "cat <(echo hi)\necho after\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "rc={code:?} stderr={stderr}");
    assert_eq!(stdout, "hi\nafter\n", "stdout: {stdout:?}");
}

/// The word-attached form through the stdin `-s` reader: rubash#463 fixed
/// the gather-feeder path; the close-char oracle arm this issue adds fixes
/// the stdin reader too (GNU: `outer/dev/fd/63`).
#[test]
fn word_attached_form_through_stdin_reader() {
    let script = "echo outer<(echo\ninner\n)\necho after\n";
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "rc={code:?} stderr={stderr}");
    assert_eq!(stdout, "outer/dev/fd/63\nafter\n", "stdout: {stdout:?}");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
}

/// A quoted `<(` is word data: no join, no capture of the next line.
#[test]
fn quoted_opener_is_word_data() {
    let script = "echo 'a<('\necho done\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "rc={code:?} stderr={stderr}");
    assert_eq!(stdout, "a<(\ndone\n", "stdout: {stdout:?}");
}

/// Closed single-line shapes stay byte-identical (no regression on the
/// forms the #463 suite already locks).
#[test]
fn complete_single_line_forms_unchanged() {
    for script in [
        "echo <(echo hi)\n",
        "cat <(echo hi)\n",
        "echo >(echo hi)\n",
        "cat <(echo hi) | tr a-z A-Z\n",
    ] {
        let (stdout, stderr, code) = rubash_file(script);
        assert_eq!(code, Some(0), "script {script:?}: {stderr}");
        assert!(!stdout.is_empty(), "script {script:?}: no output");
    }
}

/// An introducer ENDING a line (the substitution opens with nothing after
/// `(`) joins the same way — GNU: `hi` + `done`.
#[test]
fn introducer_at_line_end_joins() {
    let script = "cat <(\necho hi\n)\necho done\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "rc={code:?} stderr={stderr}");
    assert_eq!(stdout, "hi\ndone\n", "stdout: {stdout:?}");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
}

/// Parse-only acceptance (`-n`) keeps passing for the space-separated form
/// (the #463 boundary table's parse-acceptance row).
#[test]
fn parse_only_acceptance_unchanged() {
    let script = "cat <(\necho hi\n)\necho done\n";
    let (_, stderr, code) = rubash_args(script, &["-n"]);
    assert_eq!(code, Some(0), "rc={code:?} stderr={stderr}");
    assert!(stderr.is_empty(), "stderr: {stderr}");
}
