//! Issue rubash#565: the empty function-head paren — `WORD (` followed by
//! end of line — names the WRONG offending token. GNU's reader hands yacc
//! the physical line's NEWLINE token after the shifted `(' (the function_def
//! production `WORD '(' ')'` is what shifted it, parse.y:1054 via the
//! read_token classification at parse.y:3774-3790 that keeps PST_SUBSHELL
//! off after a WORD), and THAT token is rejected:
//! `syntax error near unexpected token `newline'' + print_offending_line of
//! the `(' line (parse.y:6814). rubash's lexer emits no terminator token
//! for a script's final line, so the parser saw the `(' itself as the last
//! token and named it.
//!
//! Cell family: the #563 C4b neighbours (`[ ( 1 -eq 1` names `1`, `[ ( ]`
//! names `]`, `[ ( | cat` names `|`, `echo a b (` names `(` — with two+ words
//! there is no `(' production to shift, so the `(' itself is the offender).
//! `x=1 (` is NOT a GNU cell: rubash#452 adjudicated assignment-prefix +
//! subshell as accepted (real GNU 5.3.0 rejects even the raw corpus shapes —
//! `x=a@(b|c)`, `pat=-*@(...)`, `x=1 (echo hi)` all rc 2), so the reader must
//! keep reading after `x=1 (`; this file pins that boundary in
//! c4b_neighbours_keep_their_offenders below. This issue is ONLY the
//! nothing-after-the-shifted-paren shape: `[ (` NL, `f (` NL, `echo (` NL,
//! and the same inputs at EOF without the trailing newline (GNU `bash -c
//! '[ ('` still names `newline`).
//!
//! Oracle: WSL GNU bash 5.3.0(1)-release stdin/-c probes, 2026-10-12.

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_stdin(script: &str) -> (String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script");
    let output = child.wait_with_output().expect("wait rubash");
    // The stdin driver binds argv[0] as the diagnostic program name, so the
    // harness normalizes the test executable's path to `bash` exactly like
    // the issue551 harness (GNU's own program name is `bash`).
    let exe = env!("CARGO_BIN_EXE_rubash").to_string();
    (
        String::from_utf8_lossy(&output.stderr)
            .replace(&exe, "bash")
            .replace(&exe.replace('/', "\\"), "bash"),
        output.status.code(),
    )
}

/// The issue's shape: the test-builtin word shifts `(` expecting `)`, the
/// physical line's NEWLINE arrives and is the rejected token.
#[test]
fn bracket_paren_empty_body_names_newline() {
    let (stderr, code) = rubash_stdin("[ (\n");
    assert_eq!(
        stderr,
        "bash: line 1: syntax error near unexpected token `newline'\n\
         bash: line 1: `[ ('\n",
        "GNU names the virtual newline token"
    );
    assert_eq!(code, Some(2));
}

/// A plain command word behaves identically (the WORD before `(` is not
/// special — parse.y:3779 only decides subshell vs function-def by
/// last_read_token).
#[test]
fn command_word_paren_empty_body_names_newline() {
    let (stderr, code) = rubash_stdin("echo (\n");
    assert_eq!(
        stderr,
        "bash: line 1: syntax error near unexpected token `newline'\n\
         bash: line 1: `echo ('\n"
    );
    assert_eq!(code, Some(2));
}

/// The function-name shape (`f (` + newline) rejects the newline the same
/// way — GNU's function_def parens admit no newline between them.
#[test]
fn function_name_paren_empty_body_names_newline() {
    let (stderr, code) = rubash_stdin("f (\n");
    assert_eq!(
        stderr,
        "bash: line 1: syntax error near unexpected token `newline'\n\
         bash: line 1: `f ('\n"
    );
    assert_eq!(code, Some(2));
}

/// EOF without the trailing newline (`-c` shape) still names `newline` —
/// GNU's end-of-input reports the virtual newline token.
#[test]
fn eof_without_newline_still_names_newline() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "[ ("])
        .output()
        .expect("run rubash -c");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("syntax error near unexpected token `newline'"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("`[ ('"), "stderr: {stderr}");
    assert_eq!(output.status.code(), Some(2));
}

/// #563's C4b cells keep their exact offenders: a WORD, an operator or the
/// two-words/assignment-prefix shapes never shift the paren, so the `(' or
/// the following token is named — never the virtual newline.
#[test]
fn c4b_neighbours_keep_their_offenders() {
    let (stderr, code) = rubash_stdin("[ ( 1 -eq 1\n");
    assert_eq!(
        stderr,
        "bash: line 1: syntax error near unexpected token `1'\n\
         bash: line 1: `[ ( 1 -eq 1'\n"
    );
    assert_eq!(code, Some(2));

    let (stderr, code) = rubash_stdin("[ ( ]\n");
    assert_eq!(
        stderr,
        "bash: line 1: syntax error near unexpected token `]'\n\
         bash: line 1: `[ ( ]'\n"
    );
    assert_eq!(code, Some(2));

    let (stderr, code) = rubash_stdin("[ ( | cat\n");
    assert_eq!(
        stderr,
        "bash: line 1: syntax error near unexpected token `|'\n\
         bash: line 1: `[ ( | cat'\n"
    );
    assert_eq!(code, Some(2));

    // `echo a b (`: two+ words — no `(' production to shift, so the `('
    // itself is the offender (GNU probe agrees).
    let (stderr, code) = rubash_stdin("echo a b (\n");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(2));

    // `x=1 (`: the assignment prefix committed simple_command — no shifted
    // function head, so the #565 naming fix must NOT reach this shape. The
    // pre-merge #452 adjudication joined the line waiting for `)' and
    // reported `unexpected end of file from `(' command on line 1`; the
    // master-side #516 re-adjudication (GNU p14/p16/p21 + the q-series)
    // instead rejects the paren itself — `syntax error near unexpected
    // token `('` + print_offending_line, rc 2. This merge carries the
    // trunk decision (suite-parity rule: fresh master and this branch
    // agree cell by cell).
    let (stderr, code) = rubash_stdin("x=1 (\n");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("`x=1 ('"), "stderr: {stderr}");
    assert_eq!(code, Some(2));
}

/// A real function definition still parses and runs: the empty paren pair
/// with a proper compound body (GNU function_body: compound_command —
/// `f () { ...; }` and the separated `f ( ) { ...; }` forms are untouched;
/// note a bare next-line simple-command body is invalid in GNU itself,
/// probe: `f ()` NL `echo body` names `echo`).
#[test]
fn real_function_definitions_unchanged() {
    for script in [
        "f () { echo body; }\nf\necho after\n",
        "f ( ) { echo body; }\nf\necho after\n",
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rubash");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(script.as_bytes())
            .expect("write script");
        let output = child.wait_with_output().expect("wait rubash");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stdout.contains("body") && stdout.contains("after"),
            "script {script:?}: stdout={stdout:?} stderr={stderr:?}"
        );
    }
}
