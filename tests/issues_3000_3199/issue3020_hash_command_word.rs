//! rubash#3020: a `#`-leading word in command position is dropped as a
//! comment even when it was quoted or produced by expansion.
//!
//! GNU rule: `#` introduces a comment only at the start of a word in the
//! SOURCE TEXT (parse.y:3630-3643, shell_getc/read_token path — discard
//! until EOL). Quoted or expansion-produced `#...` never passes the
//! lexer as a comment marker; in command position it is a command name
//! and fails as command-not-found (127). Argument position is unaffected.
//!
//! rubash root cause (fix lane): `execute_assignment_or_comment_command`
//! (src/executor/command_prepare.rs) checks the EXPANDED first word for
//! `starts_with('#')` and silently swallows the command with status 0.
//! Source-text comments never become word tokens at all (the scanner's
//! `#` arm, src/lexer/scanner.rs), so the executor arm has no valid
//! input — comment-ness is a lexical property, not an execution one.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3020-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn check_not_found(script: &str, expected_stderr: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        code,
        Some(127),
        "script: {script}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(stdout, "", "script {script} wrote to stdout");
    assert_eq!(stderr, expected_stderr, "script: {script}");
}

/// The issue's minimal form, verbatim: a double-quoted `#foo` as the
/// command word is a command name, not a comment.
#[test]
fn double_quoted_hash_word_is_a_command() {
    check_not_found("\"#foo\"\n", "case.sh: line 1: #foo: command not found\n");
}

/// Single-quoted form: quoting provenance does not matter, the word is
/// data either way.
#[test]
fn single_quoted_hash_word_is_a_command() {
    check_not_found("'#foo'\n", "case.sh: line 1: #foo: command not found\n");
}

/// A quoted hash word still takes arguments (it is a command, so the
/// argument is never the failure point).
#[test]
fn quoted_hash_word_with_argument() {
    check_not_found(
        "\"#foo\" some-arg\n",
        "case.sh: line 1: #foo: command not found\n",
    );
}

/// mvdan/sh t0162: the command word is produced by expansion
/// (`$foo#bar` with `foo` unset expands to `#bar`).
#[test]
fn expansion_produced_hash_word_is_a_command() {
    check_not_found(
        "$foo#bar foo#$bar\n",
        "case.sh: line 1: #bar: command not found\n",
    );
}

/// mvdan/sh t0163: the command word is produced by a command
/// substitution plus a literal `#` tail (`$(foo)#bar` -> `#bar`); the
/// inner failure is reported first, then the outer name.
#[test]
fn comsub_produced_hash_word_is_a_command() {
    check_not_found(
        "$(foo)#bar\n",
        "case.sh: line 1: foo: command not found\ncase.sh: line 1: #bar: command not found\n",
    );
}

/// Chain: the hash word is stored in a variable on one line and used
/// as the command on the next — the 127 must be observable via `$?`.
#[test]
fn hash_word_via_variable_reports_127_to_next_command() {
    let (stdout, stderr, code) = rubash_file("x=\"#foo\"\n$x\necho \"rc-was=$?\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rc-was=127\n");
    assert_eq!(stderr, "case.sh: line 2: #foo: command not found\n");
}

/// Control: a bare `#foo` line IS a source-text comment — silence,
/// status 0, and the next line runs.
#[test]
fn bare_hash_line_is_a_comment() {
    let (stdout, stderr, code) = rubash_file("#foo\necho after-bare\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "after-bare\n");
    assert_eq!(stderr, "");
}

/// Control (issue's own): argument position was never broken.
#[test]
fn hash_word_in_argument_position_prints() {
    let (stdout, stderr, code) = rubash_file("echo \"#foo\"\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "#foo\n");
    assert_eq!(stderr, "");
}

/// Control: a `#` in the middle of a word is not a comment either.
#[test]
fn mid_word_hash_is_not_a_comment() {
    let (stdout, stderr, code) = rubash_file("echo foo#bar\n");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "foo#bar\n");
    assert_eq!(stderr, "");
}
