//! rubash#2783 — a syntax error inside a `$(...)` body echoes the
//! PHYSICAL line of the enclosing input that contains the
//! offending token (GNU print_offending_line, parse.y:6814, over
//! shell_input_line), never the stored body fragment. rubash used
//! to echo the fragment (`if then`) because the body parse only
//! carried the body text. Expectations measured against GNU
//! bash 5.3.20 (2026-11-08).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    run_file_env(tag, script.as_bytes(), &[])
}

fn run_file_env(tag: &str, script: &[u8], extra_env: &[(&str, &str)]) -> Run {
    let dir: PathBuf = std::env::temp_dir().join(format!("rubashP1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
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
fn issue2783_comsub_body_error_echoes_enclosing_line() {
    let run = run_file("echo-l1", "echo $(if then)\n");
    assert_run(
        "echo-l1",
        &run,
        "",
        "case.sh: line 1: syntax error near unexpected token `then' while looking for matching `)'\n\
         case.sh: line 1: `echo $(if then)'\n",
        2,
    );
}

#[test]
fn issue2783_comsub_body_error_echoes_enclosing_line_2() {
    let run = run_file("echo-l2", "x=1\necho $(if then)\n");
    assert_run(
        "echo-l2",
        &run,
        "",
        "case.sh: line 2: syntax error near unexpected token `then' while looking for matching `)'\n\
         case.sh: line 2: `echo $(if then)'\n",
        2,
    );
}

#[test]
fn issue2783_comsub_multiline_body_echoes_token_line() {
    // The body spans two physical lines; the offending token sits on
    // line 2, and GNU echoes that physical line (`then)`) verbatim.
    let run = run_file("echo-ml", "echo $(if\nthen)\n");
    assert_run(
        "echo-ml",
        &run,
        "",
        "case.sh: line 2: syntax error near unexpected token `then' while looking for matching `)'\n\
         case.sh: line 2: `then)'\n",
        2,
    );
}

#[test]
fn issue2783_nested_comsub_body_error_echoes_enclosing_line() {
    let run = run_file("echo-nest", "echo $(echo $(if then))\n");
    assert_run(
        "echo-nest",
        &run,
        "",
        "case.sh: line 1: syntax error near unexpected token `then' while looking for matching `)'\n\
         case.sh: line 1: `echo $(echo $(if then))'\n",
        2,
    );
}
