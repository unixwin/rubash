//! Issue rubash#2467: a standalone all-digit WORD followed by whitespace
//! and then `<<` was fused into the heredoc's io_number and swallowed
//! from the argument list (`head -n 3 <<EOF` lost the `3`, so `head`
//! failed with "option requires an argument -- 'n'"; `echo 3 <<X`
//! printed an empty line).
//!
//! GNU parse.y read_token_word() (got_token, parse.y:5725-5741): an
//! all-digit token is a NUMBER only when the character that ENDED the
//! token is `<` or `>` (or the previous token was `<`/`&`). A
//! whitespace-terminated digit token is a plain WORD, so the grammar's
//! `NUMBER LESS_LESS WORD` form (parse.y:629) can only engage for the
//! fused spelling `3<<X`. Every expectation below was measured against
//! the GNU bash 5.3.20 oracle with the same scripts.
//!
//! The negative controls pin the shapes that must stay byte-identical:
//! the fused io_number (`3<<Y`, `exec 3<<X`, `exec 3<<-X`), whitespace
//! before `<` / `>` / `<<<`, and a `<&` dup ahead of a fused fd heredoc.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2467-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn collect(output: std::process::Output) -> Run {
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    collect(output)
}

fn rubash_script(script: &str, dir: &Path) -> Run {
    let path = dir.join("case.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    collect(output)
}

// ---------------------------------------------------------------------
// Positive cases: the digit word is an argument, the heredoc is fd 0.
// ---------------------------------------------------------------------

#[test]
fn issue2467_head_argument_survives_spaced_heredoc() {
    let dir = tempdir("head");
    let run = rubash_script("head -n 3 <<EOF\na\nb\nc\nd\nEOF\n", &dir);
    assert_eq!(run.stdout, "a\nb\nc\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}

#[test]
fn issue2467_echo_number_word_survives_spaced_heredoc() {
    let dir = tempdir("echo");
    let run = rubash_c("echo 3 <<X\nX\n", &dir);
    assert_eq!(run.stdout, "3\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}

#[test]
fn issue2467_multi_digit_word_survives_spaced_heredoc() {
    let dir = tempdir("multi");
    let run = rubash_c("echo 42 <<X\nX\n", &dir);
    assert_eq!(run.stdout, "42\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}

#[test]
fn issue2467_number_word_survives_spaced_heredoc_dash() {
    let dir = tempdir("dash");
    let run = rubash_c("echo 3 <<-X\nX\n", &dir);
    assert_eq!(run.stdout, "3\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}

#[test]
fn issue2467_number_word_survives_spaced_quoted_delimiter() {
    let dir = tempdir("quoted");
    let run = rubash_c("echo 3 <<'X'\nbody\nX\n", &dir);
    assert_eq!(run.stdout, "3\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}

#[test]
fn issue2467_number_word_survives_spaced_heredoc_in_pipeline() {
    let dir = tempdir("pipe");
    let run = rubash_c("echo 3 <<X | cat\nX\n", &dir);
    assert_eq!(run.stdout, "3\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}

// ---------------------------------------------------------------------
// Negative controls: byte-identical before and after the fix.
// ---------------------------------------------------------------------

#[test]
fn issue2467_adjacent_io_number_heredoc_unchanged() {
    let dir = tempdir("adjacent");
    let run = rubash_c("echo 3<<Y\nY\n", &dir);
    assert_eq!(run.stdout, "\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}

#[test]
fn issue2467_spaced_input_redirect_unchanged() {
    let dir = tempdir("input");
    let run = rubash_c("head -n 3 < /dev/null\necho rc=$?\n", &dir);
    assert_eq!(run.stdout, "rc=0\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}

#[test]
fn issue2467_spaced_output_redirect_unchanged() {
    let dir = tempdir("output");
    let run = rubash_c("echo 3 > out.txt\n", &dir);
    assert_eq!(run.stdout, "", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    let written = std::fs::read_to_string(dir.join("out.txt")).expect("read out.txt");
    assert_eq!(written, "3\n");
}

#[test]
fn issue2467_spaced_herestring_unchanged() {
    let dir = tempdir("herestring");
    let run = rubash_c("head -n 3 <<< ignored\n", &dir);
    assert_eq!(run.stdout, "ignored\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}

#[test]
fn issue2467_fused_fd_heredoc_via_exec_unchanged() {
    let dir = tempdir("fused");
    let run = rubash_script("exec 3<<X\nbodyline\nX\ncat <&3\n", &dir);
    assert_eq!(run.stdout, "bodyline\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}

#[test]
fn issue2467_fused_fd_dash_heredoc_via_exec_unchanged() {
    let dir = tempdir("fuseddash");
    let run = rubash_script("exec 3<<-X\n\tbodyline\n\tX\ncat <&3\n", &dir);
    assert_eq!(run.stdout, "bodyline\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}

#[test]
fn issue2467_dup_then_fused_fd_heredoc_unchanged() {
    let dir = tempdir("dup");
    let run = rubash_script("exec 3</dev/null\ncat <&3 3<<X\nX\necho rc=$?\n", &dir);
    assert_eq!(run.stdout, "rc=0\n", "stderr: {}", run.stderr);
    assert!(
        !run.stderr.contains("Bad file descriptor"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
}
