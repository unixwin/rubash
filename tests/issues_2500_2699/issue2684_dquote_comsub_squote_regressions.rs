//! rubash#2684 — a single-quoted double quote inside a command
//! substitution that itself sits inside double quotes:
//! `echo "$(echo '"')"` must print `"`.
//!
//! GNU reads the comsub body with its own reader (parse.y:4451
//! parse_comsub): inside the body, `'…'` quoting is the body's own
//! business and the enclosing double quote does not leak into it.
//! These pins guard the shape family (assignment, surrounding
//! text, nesting, backticks, `${…:-…}`) against the lexical
//! scanner losing that boundary again.
//!
//! Measured against GNU bash 5.3.20 (2026-11-08), byte for byte
//! (inside the body's single quotes, `\"` stays two bytes:
//! backslash + quote). These are pure guard pins: on the lane's
//! base (88047f50) all 10 cells already pass (lane REPORT,
//! logs/base-pins2.log) and this group changes no product code
//! for this issue — the pins lock the boundary against regress.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2684-{tag}-{}", std::process::id()));
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

fn assert_prints(tag: &str, script: &str, expected_stdout: &str) {
    let run = run_file(tag, script);
    assert_eq!(run.stdout, expected_stdout, "{tag}: stdout");
    assert_eq!(run.stderr, "", "{tag}: stderr");
    assert_eq!(run.code, Some(0), "{tag}: rc");
}

#[test]
fn issue2684_issue_shape() {
    assert_prints("issue", "echo \"$(echo '\"')\"\n", "\"\n");
}

#[test]
fn issue2684_assignment_form() {
    assert_prints("assign", "x=\"$(echo '\"')\"\necho \"[$x]\"\n", "[\"]\n");
}

#[test]
fn issue2684_surrounding_text() {
    assert_prints("text", "echo \"pre $(echo '\"') post\"\n", "pre \" post\n");
}

#[test]
fn issue2684_single_quotes_inside_body() {
    assert_prints("sq", "echo \"$(echo \"'x'\")\"\n", "'x'\n");
}

#[test]
fn issue2684_dquote_inside_single_quotes_in_body() {
    assert_prints("mixed", "echo \"$(echo 'a\"b')\"\n", "a\"b\n");
}

#[test]
fn issue2684_escaped_dquote_in_body() {
    // Inside the body's single quotes, `\"` is literal backslash +
    // quote (no DQ-layer reduction happens for `$(` bodies), so
    // GNU prints both bytes.
    assert_prints("esc", "echo \"$(echo '\\\"')\"\n", "\\\"\n");
}

#[test]
fn issue2684_unquoted_form() {
    assert_prints("bare", "echo $(echo '\"')\n", "\"\n");
}

#[test]
fn issue2684_backtick_form() {
    assert_prints("bt", "echo \"`echo '\\\"'`\"\n", "\"\n");
}

#[test]
fn issue2684_nested_comsub() {
    assert_prints("nested", "echo \"$(echo \"$(echo '\\\"')\")\"\n", "\\\"\n");
}

#[test]
fn issue2684_default_word_form() {
    assert_prints("default", "echo \"${x:-$(echo '\\\"')}\"\n", "\\\"\n");
}
