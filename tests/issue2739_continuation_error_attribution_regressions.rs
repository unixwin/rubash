//! rubash#2739 — a list operator that OPENS a line (the previous line
//! ended mid-pipeline / mid-list) is a yyerror at THAT token: GNU
//! reports it at its own line, with that physical line echoed
//! (parse.y report_syntax_error / print_offending_line). rubash's
//! token-action arm built a line-less, source-less message node, so
//! the diagnostic blamed the previous line and dropped the echo.
//! Expectations measured against GNU bash 5.3.20 (script files and
//! `-c` strings, 2026-11-04).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2739-{tag}-{}", std::process::id()));
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
fn issue2739_andand_opening_line_two_file() {
    // The completed first line runs (GNU reads complete commands
    // incrementally); the stray `&&` is blamed on ITS line, echoed.
    let run = run_file("andand", "echo two\n&& echo three\n");
    assert_run(
        "andand",
        &run,
        "two\n",
        "case.sh: line 2: syntax error near unexpected token `&&'\n\
         case.sh: line 2: `&& echo three'\n",
        2,
    );
}

#[test]
fn issue2739_andand_after_pipe_continuation_file() {
    let run = run_file("pipe", "echo two |\n&& echo three\n");
    assert_run(
        "pipe",
        &run,
        "",
        "case.sh: line 2: syntax error near unexpected token `&&'\n\
         case.sh: line 2: `&& echo three'\n",
        2,
    );
}

#[test]
fn issue2739_andand_opening_line_two_dash_c() {
    let run = run_c("echo two\n&& echo three");
    assert_run(
        "dash-c",
        &run,
        "two\n",
        "bash: -c: line 2: syntax error near unexpected token `&&'\n\
         bash: -c: line 2: `&& echo three'\n",
        2,
    );
}

#[test]
fn issue2739_trailing_andand_continuation_still_runs() {
    // Control: the operator at the END of a line is a continuation,
    // not an error.
    let run = run_file("ctrl", "echo two &&\necho three\n");
    assert_run("ctrl", &run, "two\nthree\n", "", 0);
}
