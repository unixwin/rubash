//! rubash#3647 — `syntax error near unexpected token `!'` in reserved
//! position (after a pipe) did not echo the offending source line.
//!
//! GNU model (bash 5.2.21, verified by oracle probe on the box):
//! - `echo hi | ! true` -> `bash: -c: line 1: syntax error near unexpected
//!   token `!'` + `bash: -c: line 1: `echo hi | ! true'`, rc 2.
//! - The echo happens for every "near unexpected token" report and echoes
//!   the physical line verbatim (leading/trailing spaces and a trailing
//!   comment survive); the echoed line is the one at the reported line
//!   number (multi-line input echoes line 2 only, after line 1 ran).
//! - "syntax error: unexpected end of file" reports echo nothing.
//! - Script files echo the same way (`probe.sh: line 1: `...'`).
//!
//! Root cause: the `!`-after-pipe arm in `src/parser/token_actions.rs`
//! (parse.y:1408-1413, rubash#360) hand-rolled a
//! `__RUBASH_PARSE_ERROR_NEAR__` node with the token + line but no
//! `__RUBASH_PARSE_SOURCE__` assignment, so the executor's NEAR arm had
//! no line to echo. Every sibling NEAR producer attaches the source via
//! `offending_line_text`; this arm was the only one that did not.
//!
//! The `-c` assertions match on line suffixes, not the full stderr text:
//! the diagnostic prefix carries argv[0], which is the test binary path
//! under `cargo test` and `bash` under GNU (see the pre-existing stale
//! `bash:` expectations in `tests/issue360_parser_overlax_regressions.rs`).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3647-{tag}-{}", std::process::id()));
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

fn run_c(script: &str) -> (String, String, i32) {
    let dir = tempdir("c");
    run_rubash(&["-c", script], &dir)
}

fn run_file(script: &str) -> (String, String, i32) {
    let dir = tempdir("file");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write probe.sh");
    run_rubash(&["probe.sh"], &dir)
}

/// Assert stderr is exactly [diagnostic, source-echo] for a `-c` run:
/// line 0 ends with the diagnostic, line 1 is the verbatim echo of
/// `source` at `line`, both under the `-c: line N:` prefix.
fn assert_echo(stderr: &str, line: usize, diagnostic: &str, source: &str) {
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 2, "stderr: {stderr}");
    assert!(
        lines[0].ends_with(&format!(": -c: line {line}: {diagnostic}")),
        "diagnostic line: {stderr}"
    );
    assert!(
        lines[1].ends_with(&format!(": -c: line {line}: `{source}'")),
        "echo line: {stderr}"
    );
}

#[test]
fn seed_bang_after_pipe_echoes_source_line() {
    let (stdout, stderr, rc) = run_c("echo hi | ! true");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    assert_echo(
        &stderr,
        1,
        "syntax error near unexpected token `!'",
        "echo hi | ! true",
    );
}

#[test]
fn bang_after_pipe_second_stage_echoes() {
    let (stdout, stderr, rc) = run_c("! true | ! true");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    assert_echo(
        &stderr,
        1,
        "syntax error near unexpected token `!'",
        "! true | ! true",
    );
}

#[test]
fn bare_bang_after_pipe_echoes() {
    let (stdout, stderr, rc) = run_c("echo hi | !");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    assert_echo(
        &stderr,
        1,
        "syntax error near unexpected token `!'",
        "echo hi | !",
    );
}

#[test]
fn bang_after_pipe_mid_pipeline_echoes() {
    let (stdout, stderr, rc) = run_c("echo hi | ! true | cat");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    assert_echo(
        &stderr,
        1,
        "syntax error near unexpected token `!'",
        "echo hi | ! true | cat",
    );
}

#[test]
fn bang_after_pipe_pipeerr_echoes() {
    let (stdout, stderr, rc) = run_c("echo hi |& ! true");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    assert_echo(
        &stderr,
        1,
        "syntax error near unexpected token `!'",
        "echo hi |& ! true",
    );
}

#[test]
fn multiline_echoes_offending_line_only() {
    let (stdout, stderr, rc) = run_c("echo a\necho hi | ! true\necho b");
    assert_eq!(stdout, "a\n", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    assert_echo(
        &stderr,
        2,
        "syntax error near unexpected token `!'",
        "echo hi | ! true",
    );
}

#[test]
fn echo_preserves_spacing_and_comment() {
    let (stdout, stderr, rc) = run_c("  echo hi | ! true  ");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    assert_echo(
        &stderr,
        1,
        "syntax error near unexpected token `!'",
        "  echo hi | ! true  ",
    );
    let (stdout, stderr, rc) = run_c("echo hi | ! true # comment");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    assert_echo(
        &stderr,
        1,
        "syntax error near unexpected token `!'",
        "echo hi | ! true # comment",
    );
}

#[test]
fn script_file_echoes_source_line() {
    let (stdout, stderr, rc) = run_file("echo hi | ! true\n");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 2, "stderr: {stderr}");
    assert!(
        lines[0].ends_with("probe.sh: line 1: syntax error near unexpected token `!'"),
        "diagnostic line: {stderr}"
    );
    assert!(
        lines[1].ends_with("probe.sh: line 1: `echo hi | ! true'"),
        "echo line: {stderr}"
    );
}

#[test]
fn eof_error_has_no_echo() {
    let (stdout, stderr, rc) = run_c("echo hi |");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 1, "stderr: {stderr}");
    assert!(
        lines[0].ends_with(": -c: line 2: syntax error: unexpected end of file"),
        "stderr: {stderr}"
    );
}

#[test]
fn legal_bang_positions_stay_legal() {
    // No `-n` here, so the codes are the real inverted ones: `! true`
    // is 1, `! ! true` is 0, a pipeline's code is its last stage's.
    for (script, rc) in [
        ("! true", 1),
        ("true && ! true", 1),
        ("true; ! true", 1),
        ("! ! true", 0),
        ("true | { ! true; }", 1),
    ] {
        let (stdout, stderr, got) = run_c(script);
        assert_eq!(stdout, "", "script {script:?} stderr: {stderr}");
        assert_eq!(stderr, "", "script {script:?}");
        assert_eq!(got, rc, "script {script:?} stderr: {stderr}");
    }
}
