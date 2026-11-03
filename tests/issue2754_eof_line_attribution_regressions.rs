//! rubash#2754 — a list left dangling at end of input (`echo a |`,
//! `echo b &&`) reports `syntax error: unexpected end of file` at
//! the END-OF-INPUT line (parse.y yyerror EOF path: the line the
//! reader reached), no matter how many commands the list spans.
//! rubash's and/or + pipeline folds attached the EOF marker to the
//! FIRST command of the list (and multi-command dangling lists were
//! not folded at all — they executed with rc 0), so the report named
//! first-line + 1. Expectations measured against GNU bash 5.3.20
//! (2026-11-04).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2754-{tag}-{}", std::process::id()));
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
fn issue2754_two_stage_dangling_pipeline() {
    let run = run_file("a", "echo a |\necho b |\n");
    assert_run(
        "a",
        &run,
        "",
        "case.sh: line 3: syntax error: unexpected end of file\n",
        2,
    );
}

#[test]
fn issue2754_two_command_dangling_andor() {
    let run = run_file("b", "echo a &&\necho b &&\n");
    assert_run(
        "b",
        &run,
        "",
        "case.sh: line 3: syntax error: unexpected end of file\n",
        2,
    );
}

#[test]
fn issue2754_single_dangling_pipeline() {
    let run = run_file("c", "echo a |\n");
    assert_run(
        "c",
        &run,
        "",
        "case.sh: line 2: syntax error: unexpected end of file\n",
        2,
    );
}

#[test]
fn issue2754_three_stage_dangling_pipeline() {
    let run = run_file("d", "true |\ntrue |\ntrue |\n");
    assert_run(
        "d",
        &run,
        "",
        "case.sh: line 4: syntax error: unexpected end of file\n",
        2,
    );
}

#[test]
fn issue2754_dangling_andor_dash_c() {
    let run = run_c("echo a &&");
    assert_run(
        "andor-c",
        &run,
        "",
        "bash: -c: line 2: syntax error: unexpected end of file\n",
        2,
    );
}
