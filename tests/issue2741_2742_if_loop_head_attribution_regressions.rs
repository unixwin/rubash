//! rubash#2741 / rubash#2742 — reserved words carry force only at
//! command position (parse.y CHECK_FOR_RESERVED_WORD /
//! reserved_word_acceptable), so inside an `if`/`while` condition LIST
//! the word `then`/`do` is an ordinary argument until a command
//! position actually arrives:
//!
//! #2741: `if true then echo hi; fi` — `then` is echo's argument, the
//!   condition list is still unterminated at `fi`, and GNU blames
//!   `fi`. rubash's frame scan treated any `then` as the frame
//!   boundary and blamed `then`. A `then` dangling after a connector
//!   (`if a && then`) IS at command position and is blamed itself.
//! #2742: `while do done` — the token in `do`'s place after an EMPTY
//!   condition scan is `do` itself (a boundary keyword arriving where
//!   a command was expected); rubash blamed the following `done`.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-04).

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_c(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_run(tag: &str, run: &Run, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(run.stdout, stdout, "{tag}: stdout");
    assert_eq!(run.stderr, stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(code), "{tag}: rc");
}

#[test]
fn issue2741_then_as_condition_argument_blames_fi() {
    let run = run_c("if true then echo hi; fi");
    assert_run(
        "r3a",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `fi'\n\
         bash: -c: line 1: `if true then echo hi; fi'\n",
        2,
    );
}

#[test]
fn issue2741_then_as_condition_argument_semicolon_form() {
    let run = run_c("if true then; fi");
    assert_run(
        "r3b",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `fi'\n\
         bash: -c: line 1: `if true then; fi'\n",
        2,
    );
}

#[test]
fn issue2741_then_argument_false_condition() {
    let run = run_c("if false then echo hi; fi; echo after=$?");
    assert_run(
        "r3c",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `fi'\n\
         bash: -c: line 1: `if false then echo hi; fi; echo after=$?'\n",
        2,
    );
}

#[test]
fn issue2741_then_at_command_position_after_connector() {
    // After `&&` the list dangles: `then` sits at command position
    // and is itself the error token.
    let run = run_c("if a && then; fi");
    assert_run(
        "and-then",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `then'\n\
         bash: -c: line 1: `if a && then; fi'\n",
        2,
    );
}

#[test]
fn issue2741_fi_as_condition_argument_is_legal() {
    // Control: `fi` as a plain argument is not a closer either.
    let run = run_c("if true fi; then echo yes; fi");
    assert_run("fi-arg", &run, "yes\n", "", 0);
}

#[test]
fn issue2742_while_do_done_blames_do() {
    let run = run_c("while do done");
    assert_run(
        "r4a",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `do'\n\
         bash: -c: line 1: `while do done'\n",
        2,
    );
}

#[test]
fn issue2742_until_do_done_blames_do() {
    let run = run_c("until do done");
    assert_run(
        "r4b",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `do'\n\
         bash: -c: line 1: `until do done'\n",
        2,
    );
}

#[test]
fn issue2742_while_do_with_body_blames_do() {
    let run = run_c("while do echo x; done");
    assert_run(
        "r4c",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `do'\n\
         bash: -c: line 1: `while do echo x; done'\n",
        2,
    );
}

#[test]
fn issue2741_2742_wellformed_controls() {
    let run = run_c("if true; then echo yes; fi");
    assert_run("if-ctrl", &run, "yes\n", "", 0);
    let run = run_c("i=0; while [ $i -lt 2 ]; do i=$((i+1)); done; echo i=$i");
    assert_run("while-ctrl", &run, "i=2\n", "", 0);
}
