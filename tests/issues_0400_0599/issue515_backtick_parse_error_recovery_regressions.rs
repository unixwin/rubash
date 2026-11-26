//! Issue #515: a syntax error inside an old-style backtick substitution is
//! NOT fatal — GNU reports it under the `command substitution:` child name,
//! the substitution yields empty, and the script continues (rc 0).
//!
//! GNU spec (subst.c string_locale -> parse_and_execute: the backtick body
//! parses inside the substituted child at execution time; the child's
//! parser_error carries yy_input_name()=="command substitution"; the child
//! exits 2 and the parent keeps reading): `echo \`case x in\`` prints
//! `bash: command substitution: line 2: syntax error: unexpected end of
//! file from \`case' command on line 1` then an empty line (echo ran),
//! rc 0. An assignment keeps the child's status in $?
//! (`x=\`case x in\`` -> $?=2) and later commands run. rubash escalated
//! the same body error to a whole-shell exit 2 with no command output.
//! Same semantic point as huck#215 (fixed there: empty subst, rc 0).
//!
//! The body parses NEWLINE-TERMINATED (GNU's line reader), so a 1-line
//! unterminated body reports its EOF on the NEXT line — a backtick on
//! script line N reports `line N+1 ... on line N` (measured WSL GNU 5.3.0:
//! backtick on line 2 -> `line 3 ... on line 2`).
//!
//! The `$(`-body shape is the OPPOSITE and stays fatal: the body is parsed
//! by the outer read, so `echo $(case x in)` kills the script with rc 2
//! (the script's later lines never run).
//!
//! Rust semantic owner: executor/command_substitution_values.rs (ambient
//! backtick window), executor/public_accessors.rs (latch + comsub prefix +
//! EOF_COMPOUND render), executor/command_execute.rs (escalation skip),
//! executor/ast_exec.rs (loop containment skip), embedded_mutations.rs /
//! command_substitution.rs (newline-terminated body parse).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue515-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue515 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let code = output.status.code();
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The corpus shape: error under the child name, then the empty echo line,
/// and the script continues with rc 0.
#[test]
fn issue515_backtick_error_recovers_and_continues() {
    let out = run_rubash_script("echo `case x in`\necho \"rc=$?\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "\nrc=0\n");
    assert!(
        out.stderr.ends_with(
            "command substitution: line 2: syntax error: unexpected end of file from `case' command on line 1\n"
        ),
        "got {:?}",
        out.stderr
    );
    assert!(!out.stderr.contains("syntax error in command substitution"));
}

/// A backtick on script line 2 reports the EOF one line later (GNU:
/// `line 3 ... on line 2`).
#[test]
fn issue515_eof_line_inherits_script_counter() {
    let out = run_rubash_script("true\necho `case x in`\necho done\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "\ndone\n");
    assert!(
        out.stderr.ends_with(
            "command substitution: line 3: syntax error: unexpected end of file from `case' command on line 2\n"
        ),
        "got {:?}",
        out.stderr
    );
}

/// An assignment keeps the failed child's status 2 in $?, and the script
/// continues (GNU `after=2`, `done`).
#[test]
fn issue515_assignment_keeps_child_status() {
    let out = run_rubash_script("x=`case x in`\necho \"after=$?\"\necho done\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "after=2\ndone\n");
}

/// A working backtick still substitutes (guard against the recovery path
/// swallowing bodies).
#[test]
fn issue515_working_backtick_unchanged() {
    let out = run_rubash_script("echo `echo hi`\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "hi\n");
    assert_eq!(out.stderr, "");
}

/// The `$(`-body error is a read-time failure of the enclosing command:
/// the script dies rc 2 and later lines never run (unchanged fatality).
#[test]
fn issue515_dollar_paren_body_stays_fatal() {
    let out = run_rubash_script("echo $(case x in)\necho \"should-not-run=$?\"\n");
    assert_eq!(out.code, Some(2));
    assert_eq!(out.stdout, "");
}
