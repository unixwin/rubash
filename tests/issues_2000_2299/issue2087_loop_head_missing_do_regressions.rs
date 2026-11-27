//! rubash#2087 — a loop head missing `do` must produce ONLY the GNU
//! syntax error; the parser must never degrade the recognized head into
//! an executable simple command.
//!
//! GNU: the loop grammars admit exactly one continuation after the
//! head — `for_command` (parse.y:893 `FOR WORD newline_list DO ...`,
//! parse.y:921 `... IN word_list list_terminator newline_list DO ...`),
//! `select_command` (parse.y:979 / parse.y:1007 twins), and
//! parse.y:867-877 `WHILE compound_list DO compound_list DONE` (UNTIL
//! twin). After the word list's list_terminator and the newline list,
//! only `do` (or the `{` body form) may follow; the first other token is
//! the syntax-error token, and yyerror discards the whole command —
//! nothing of it executes (a `while`/`until` condition list likewise
//! runs until `do` at a command position, so a foreign closer or the
//! `done` that arrived in `do`'s place is the error token).
//!
//! rubash's hand-rolled head parsers kept collecting list words past
//! the terminator and returned None upstream, so the token loop fell
//! through to simple-command parsing and EXECUTED the head (`for:
//! command not found`, following lines ran as their own commands); the
//! while/until recovery blamed the missing `do` itself with a bare
//! `bash:`-prefixed message. All expectations measured against GNU
//! bash 5.3.20 with script files (2026-10-29).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2087-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// A broken head: GNU's single syntax error, rc 2, and NOTHING executes
/// (empty stdout — no `command not found`, no body fragments, no AFTER).
fn assert_gnu_error(tag: &str, script: &str, expected_stderr: &str) {
    let run = run(tag, script);
    assert_eq!(
        run.stdout, "",
        "{tag}: stdout must stay empty: {:?}",
        run.stdout
    );
    assert_eq!(run.stderr, expected_stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(2), "{tag}: rc");
}

#[test]
fn issue2087_for_missing_do_newline_form() {
    // The filed seed: `for i in 1 2` + `done` used to print
    // `for: command not found` before the syntax error.
    assert_gnu_error(
        "for-seed",
        "for i in 1 2\ndone\necho AFTER\n",
        "case.sh: line 2: syntax error near unexpected token `done'\n\
         case.sh: line 2: `done'\n",
    );
}

#[test]
fn issue2087_for_missing_do_semicolon_form() {
    assert_gnu_error(
        "for-semi",
        "for i in 1 2; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `done'\n\
         case.sh: line 1: `for i in 1 2; done'\n",
    );
}

#[test]
fn issue2087_select_missing_do_newline_form() {
    assert_gnu_error(
        "select-nl",
        "select x in a b\ndone\necho AFTER\n",
        "case.sh: line 2: syntax error near unexpected token `done'\n\
         case.sh: line 2: `done'\n",
    );
}

#[test]
fn issue2087_while_missing_do_blames_done_not_do() {
    assert_gnu_error(
        "while-nl",
        "while true\ndone\necho AFTER\n",
        "case.sh: line 2: syntax error near unexpected token `done'\n\
         case.sh: line 2: `done'\n",
    );
}

#[test]
fn issue2087_until_missing_do_blames_done_not_do() {
    assert_gnu_error(
        "until-semi",
        "until false; done\necho AFTER\n",
        "case.sh: line 1: syntax error near unexpected token `done'\n\
         case.sh: line 1: `until false; done'\n",
    );
}

#[test]
fn issue2087_for_words_after_terminator_blame_first_word() {
    // `echo x` sits where only `do` may follow the list terminator;
    // absorbing it as more list words used to run a bogus loop.
    assert_gnu_error(
        "words-after-term",
        "for i in 1 2; echo x; do echo \"iter $i\"; done\n",
        "case.sh: line 1: syntax error near unexpected token `echo'\n\
         case.sh: line 1: `for i in 1 2; echo x; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2087_for_body_fragment_never_executes() {
    // GNU blames the first body line (it is the token found where `do`
    // was expected) and the fragment never runs.
    assert_gnu_error(
        "body-frag",
        "for i in 1 2\necho INNER\ndone\n",
        "case.sh: line 2: syntax error near unexpected token `echo'\n\
         case.sh: line 2: `echo INNER'\n",
    );
}

#[test]
fn issue2087_nested_for_missing_do_blames_inner_for() {
    assert_gnu_error(
        "nested",
        "for i in 1 2\nfor j in 3 4\ndone\ndone\n",
        "case.sh: line 2: syntax error near unexpected token `for'\n\
         case.sh: line 2: `for j in 3 4'\n",
    );
}

#[test]
fn issue2087_for_no_in_words_after_terminator_blame_first_word() {
    assert_gnu_error(
        "no-in-words",
        "for i; echo x; do echo \"iter $i\"; done\n",
        "case.sh: line 1: syntax error near unexpected token `echo'\n\
         case.sh: line 1: `for i; echo x; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2087_while_foreign_closer_blames_closer() {
    assert_gnu_error(
        "while-fi",
        "while true; fi; done\n",
        "case.sh: line 1: syntax error near unexpected token `fi'\n\
         case.sh: line 1: `while true; fi; done'\n",
    );
}

#[test]
fn issue2087_for_case_terminator_in_list_blames_terminator() {
    // The SEMI_SEMI family is never a word_list word in GNU's grammar.
    assert_gnu_error(
        "dsemi",
        "for i in 1 2;; do echo \"iter $i\"; done\n",
        "case.sh: line 1: syntax error near unexpected token `;;'\n\
         case.sh: line 1: `for i in 1 2;; do echo \"iter $i\"; done'\n",
    );
}

#[test]
fn issue2087_while_unclosed_still_reports_eof_form() {
    // No closer anywhere: the EOF shape is unchanged by the recovery
    // rework (the offender scan must not invent a near-token error).
    assert_gnu_error(
        "while-eof",
        "while false\necho AFTER\n",
        "case.sh: line 3: syntax error: unexpected end of file from `while' command on line 1\n",
    );
}

#[test]
fn issue2087_wellformed_loops_still_run() {
    let out = run("ctrl-for", "for i in 1 2; do echo \"iter $i\"; done\n");
    assert_eq!(out.stdout, "iter 1\niter 2\n", "ctrl-for stdout");
    assert_eq!(out.stderr, "", "ctrl-for stderr: {}", out.stderr);
    assert_eq!(out.code, Some(0), "ctrl-for rc");

    // `done` inside the word list is a genuine list word, not the
    // closer: the terminator has not been seen yet at that point.
    let out = run(
        "ctrl-listword",
        "for i in 1 2 done; do echo \"iter $i\"; done\n",
    );
    assert_eq!(
        out.stdout, "iter 1\niter 2\niter done\n",
        "ctrl-listword stdout"
    );
    assert_eq!(out.stderr, "", "ctrl-listword stderr: {}", out.stderr);
    assert_eq!(out.code, Some(0), "ctrl-listword rc");

    let out = run(
        "ctrl-while",
        "n=0; while [ $n -lt 2 ]; do n=$((n+1)); done; echo \"n=$n\"\n",
    );
    assert_eq!(out.stdout, "n=2\n", "ctrl-while stdout");
    assert_eq!(out.stderr, "", "ctrl-while stderr: {}", out.stderr);
    assert_eq!(out.code, Some(0), "ctrl-while rc");
}
