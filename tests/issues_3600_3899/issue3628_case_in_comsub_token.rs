//! rubash#3628 — `case' inside a command substitution reported the wrong
//! unexpected token in `syntax error near unexpected token' diagnostics.
//! GNU names the token that actually arrived where the grammar expected
//! something else; rubash named a later token (`esac', `;;', `)') or the
//! end of file instead.
//!
//! GNU model (bash 5.3.20, verified by oracle probe on the box):
//! - `v=$(case x in x) done ;; esac)' -> `syntax error near unexpected
//!   token `done' while looking for matching `)'' + the echoed line, rc 2.
//! - `v=$(case x in x) in esac)'    -> `... token `in' while looking for
//!   matching `)'' + the echoed line, rc 2.
//! - `v=$(case x in x) esac ;; esac)' -> `... token `;;' while looking for
//!   matching `)'' + the echoed line, rc 2.
//! - The same wrong-token class reproduces WITHOUT a command
//!   substitution: `case x in x) done ;; esac' names `done' (not `;;'/
//!   `esac'), `case x in x) in esac' names `in' (not `esac'), and a stray
//!   `)' after the offender does not preempt it (`done ) ;; esac' names
//!   `done').
//! - `in' is a reserved word that starts no command: at command position
//!   (`in a', `echo hi | in a', `! in a') GNU reports `syntax error near
//!   unexpected token `in''; after an assignment/redirect prefix the
//!   command word position makes it a plain word (`x=1 in a' ->
//!   `in: command not found', rc 127). Argument position stays legal
//!   (`echo in', `echo done', `case x in x) echo done fi ;; esac').
//!
//! Root cause (two cooperating defects):
//! 1. `src/parser/token_actions.rs` — the command-position reserved-word
//!    rejection (`command_is_pending_pipeline_prefix`) omitted `in', so a
//!    bare `in' at command position was pushed as a command word instead
//!    of being named by yyerror like every other non-command keyword.
//!    GNU parse.y CHECK_FOR_RESERVED_WORD / the compound_list grammar
//!    (parse.y:1262) admit no production for it.
//! 2. `src/parser/case_command.rs` `case_body_end` — a lexical
//!    `done|fi|then|else|elif` break stopped the clause-body walk at those
//!    keywords without reporting them, so the offender was swallowed and a
//!    later `;;'/`esac'/`)' was blamed; it also misfired on
//!    argument-position same-spellings (`echo done fi'). Removing the
//!    break lets the real body parser judge command position, and a
//!    command-position offender (`in', `done', `fi', ...) is now reported
//!    at the first bad token like GNU's yyerror.
//!
//! Assertions match on stderr substrings / exit codes, not the full text:
//! the diagnostic prefix carries the script path, which differs between
//! the test binary and a hand-run shell.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3628-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_rubash(args: &[&str], dir: &PathBuf) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn run_file(script: &str) -> (String, String, i32) {
    let dir = tempdir("file");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write probe.sh");
    run_rubash(&["probe.sh"], &dir)
}

/// Assert a `syntax error near unexpected token `tok'` report at rc 2,
/// with `echo` (the physical line) echoed back when given.
fn assert_near(
    script: &str,
    token: &str,
    suffix: Option<&str>,
    stdout: &str,
    stderr: &str,
    rc: i32,
) {
    let diagnostic = match suffix {
        Some(s) => format!("syntax error near unexpected token `{token}' {s}"),
        None => format!("syntax error near unexpected token `{token}'"),
    };
    assert_eq!(rc, 2, "script {script:?} rc stderr: {stderr}");
    assert!(
        stderr.contains(&diagnostic),
        "script {script:?} missing `{diagnostic}' stderr: {stderr}"
    );
    if let Some(line) = script.lines().next() {
        assert!(
            stderr.contains(&format!("`{line}'")),
            "script {script:?} missing echoed line `{line}' stderr: {stderr}"
        );
    }
    let _ = stdout;
}

// ---- the issue's command-substitution reproducer ----

#[test]
fn comsub_done_names_done_not_esac() {
    let (stdout, stderr, rc) = run_file("v=$(case x in x) done ;; esac)\n");
    assert_near(
        "v=$(case x in x) done ;; esac)",
        "done",
        Some("while looking for matching `)'"),
        &stdout,
        &stderr,
        rc,
    );
}

#[test]
fn comsub_in_names_in_not_terminator() {
    let (stdout, stderr, rc) = run_file("v=$(case x in x) in esac)\n");
    assert_near(
        "v=$(case x in x) in esac)",
        "in",
        Some("while looking for matching `)'"),
        &stdout,
        &stderr,
        rc,
    );
}

#[test]
fn comsub_esac_terminator_names_next_terminator() {
    // `esac' closes the case; the `;;' after it is the stray token GNU
    // names (with the comsub matching-`)` suffix), not `esac' itself.
    let (stdout, stderr, rc) = run_file("v=$(case x in x) esac ;; esac)\n");
    assert_near(
        "v=$(case x in x) esac ;; esac)",
        ";;",
        Some("while looking for matching `)'"),
        &stdout,
        &stderr,
        rc,
    );
}

// ---- the shared case-clause-body root cause (no comsub) ----

#[test]
fn clause_body_done_names_done() {
    let (stdout, stderr, rc) = run_file("case x in x) done ;; esac\n");
    assert_near(
        "case x in x) done ;; esac",
        "done",
        None,
        &stdout,
        &stderr,
        rc,
    );
}

#[test]
fn clause_body_in_names_in() {
    let (stdout, stderr, rc) = run_file("case x in x) in esac\n");
    assert_near("case x in x) in esac", "in", None, &stdout, &stderr, rc);
}

#[test]
fn clause_body_keyword_offenders_name_themselves() {
    for (body, token) in [
        ("fi", "fi"),
        ("then", "then"),
        ("else", "else"),
        ("elif", "elif"),
        ("do", "do"),
    ] {
        let script = format!("case x in x) {body} ;; esac\n");
        let (stdout, stderr, rc) = run_file(&script);
        assert_near(&script, token, None, &stdout, &stderr, rc);
    }
}

#[test]
fn clause_body_first_offender_beats_stray_close_paren() {
    // GNU's yyerror names the FIRST token the grammar rejects: the
    // command-position `done'/`fi', not the stray `)' that follows it.
    for (script, token) in [
        ("case x in x) done ) ;; esac\n", "done"),
        ("case x in x) fi ) ;; esac\n", "fi"),
        ("case x in x) done )\n", "done"),
    ] {
        let (stdout, stderr, rc) = run_file(script);
        assert_near(script, token, None, &stdout, &stderr, rc);
    }
}

#[test]
fn clause_body_in_at_eof_names_in() {
    let (stdout, stderr, rc) = run_file("case x in x) in\n");
    assert_near("case x in x) in", "in", None, &stdout, &stderr, rc);
}

// ---- `in' command-position rejection (token_actions.rs) ----

#[test]
fn in_at_command_position_is_a_syntax_error() {
    for script in ["in a\n", "echo hi | in a\n", "! in a\n", "time in a\n"] {
        let (stdout, stderr, rc) = run_file(script);
        assert_near(script, "in", None, &stdout, &stderr, rc);
    }
}

#[test]
fn in_after_assignment_prefix_is_a_command_word() {
    // After an assignment/redirect the command word position makes `in'
    // a plain word — GNU runs it and reports `in: command not found'
    // (rc 127), not a syntax error.
    for script in ["x=1 in a\n", ">f in a\n"] {
        let (stdout, stderr, rc) = run_file(script);
        assert_eq!(rc, 127, "script {script:?} rc stderr: {stderr}");
        assert!(
            stderr.contains("in: command not found"),
            "script {script:?} stderr: {stderr}"
        );
        let _ = stdout;
    }
}

// ---- argument position must stay legal (no over-rejection) ----

#[test]
fn keyword_arguments_stay_arguments() {
    for (script, expected) in [
        ("echo in\n", "in\n"),
        ("echo done\n", "done\n"),
        // `done'/`fi' after a command word are arguments, not the
        // command-position offenders the clause-body walk used to
        // misclassify.
        ("case x in x) echo done fi ;; esac\n", "done fi\n"),
        ("case x in x) echo in esac ;; esac\n", "in esac\n"),
    ] {
        let (stdout, stderr, rc) = run_file(script);
        assert_eq!(rc, 0, "script {script:?} rc stderr: {stderr}");
        assert_eq!(stdout, expected, "script {script:?} stderr: {stderr}");
        assert_eq!(stderr, "", "script {script:?} stderr: {stderr}");
    }
}

#[test]
fn and_or_rhs_keyword_is_command_position() {
    // `&&'/`|'/`!' leave the list expecting a command, so the keyword
    // after them is the offender GNU names.
    for (script, token) in [
        ("case x in x) echo a && done ;; esac\n", "done"),
        ("case x in x) echo a | done ;; esac\n", "done"),
        ("case x in x) ! done ;; esac\n", "done"),
    ] {
        let (stdout, stderr, rc) = run_file(script);
        assert_near(script, token, None, &stdout, &stderr, rc);
    }
}
