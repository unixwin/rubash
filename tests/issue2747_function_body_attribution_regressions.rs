//! rubash#2747 — a function definition's body must be a compound
//! command (parse.y function_body). The token that arrives in the
//! body's place is the yyerror token: `f()}` blames `}`, a redirect
//! blames `>`, a plain word blames the word, and `time` — which opens
//! a PIPELINE, not a compound — is blamed itself whatever follows it.
//! rubash's body dispatch returned None upstream (blaming the `(` of
//! the definition) and its `time` arm even ACCEPTED `f() time ...`
//! as a body. Expectations measured against GNU bash 5.3.20
//! (2026-11-04).

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
fn issue2747_rbrace_body() {
    let run = run_c("f()}");
    assert_run(
        "r9a",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `}'\n\
         bash: -c: line 1: `f()}'\n",
        2,
    );
}

#[test]
fn issue2747_rbrace_body_function_keyword() {
    let run = run_c("function f()}");
    assert_run(
        "r9b",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `}'\n\
         bash: -c: line 1: `function f()}'\n",
        2,
    );
}

#[test]
fn issue2747_redirect_before_body() {
    let run = run_c("f() > /dev/null { echo hi; }");
    assert_run(
        "r9c",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `>'\n\
         bash: -c: line 1: `f() > /dev/null { echo hi; }'\n",
        2,
    );
}

#[test]
fn issue2747_word_body() {
    let run = run_c("f() echo hi");
    assert_run(
        "word",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `echo'\n\
         bash: -c: line 1: `f() echo hi'\n",
        2,
    );
}

#[test]
fn issue2747_number_body() {
    let run = run_c("f() 42");
    assert_run(
        "num",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `42'\n\
         bash: -c: line 1: `f() 42'\n",
        2,
    );
}

#[test]
fn issue2747_time_body_rejected_at_time() {
    // A timed pipeline is not a compound command: GNU rejects every
    // `f() time ...` at the `time` itself.
    let run = run_c("f() time echo ok; f");
    assert_run(
        "time",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `time'\n\
         bash: -c: line 1: `f() time echo ok; f'\n",
        2,
    );
}

#[test]
fn issue2747_bang_body_rejected_at_bang() {
    let run = run_c("f() ! { echo ok; }; f");
    assert_run(
        "bang",
        &run,
        "",
        "bash: -c: line 1: syntax error near unexpected token `!'\n\
         bash: -c: line 1: `f() ! { echo ok; }; f'\n",
        2,
    );
}

#[test]
fn issue2747_compound_body_controls() {
    let run = run_c("f() { echo ok; }; f");
    assert_run("brace", &run, "ok\n", "", 0);
    let run = run_c("f() if true; then echo ok; fi; f");
    assert_run("if", &run, "ok\n", "", 0);
}
