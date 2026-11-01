//! rubash#2441 — a `$[...]` span inside double quotes whose body
//! contains the same quote character: GNU delimits the double-quoted
//! segment BEFORE extraction (subst.c:1014 skip_double_quoted has no
//! $[...] case, so the `"` after `$[` closes the segment), and the
//! extraction over the truncated segment (subst.c:1301
//! extract_arithmetic_subst -> subst.c:1361 extract_delimited_string)
//! finds no closing `]`: it reports `bad substitution: no closing `]'`
//! in <segment through the failing `$[`> and discards the command with
//! status 1. Rubash printed the literal text with status 0.
//!
//! All expectations below were measured against GNU bash 5.3.20 (the
//! project oracle) with these exact script files.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-2441-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out: Output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

fn assert_verdict(tag: &str, script: &str, expected_fragment: &str) {
    let scratch = Scratch::new(tag);
    let (stdout, stderr, code) = scratch.run(script);
    assert_eq!(code, 1, "{tag}: exit code (stderr: {stderr})");
    assert_eq!(stdout, "", "{tag}: stdout (stderr: {stderr})");
    assert!(
        stderr.contains(expected_fragment),
        "{tag}: stderr should contain {expected_fragment:?}, got {stderr:?}"
    );
}

fn assert_script(tag: &str, script: &str, expected_stdout: &str) {
    let scratch = Scratch::new(tag);
    let (stdout, stderr, code) = scratch.run(script);
    assert_eq!(code, 0, "{tag}: exit code (stderr: {stderr})");
    assert_eq!(stdout, expected_stdout, "{tag}: stdout (stderr: {stderr})");
}

/// The filed seed, verbatim: the `"` inside the span closes the
/// segment, so extraction runs on `i=$[` alone.
#[test]
fn seed_same_quote_inside_dollar_bracket_discards_command() {
    assert_verdict(
        "seed",
        "echo \"i=$[\"1]\"]\"\n",
        "bad substitution: no closing `]' in i=$[",
    );
}

/// A complete span earlier in the same segment does not rescue the
/// failing one; the named string runs through the failing `$[`.
#[test]
fn earlier_complete_span_does_not_rescue_failing_span() {
    assert_verdict(
        "qshape",
        "echo \"q$[x]w$[\"e\"]r\"\n",
        "bad substitution: no closing `]' in q$[x]w$[",
    );
    assert_verdict(
        "vshape",
        "x=3\necho \"v=$[x+1] and $[\"2\"] end\"\n",
        "bad substitution: no closing `]' in v=$[x+1] and $[",
    );
}

/// The command is discarded, not the script: the following line runs.
#[test]
fn script_continues_after_discarded_command() {
    let scratch = Scratch::new("continue");
    let (stdout, stderr, code) = scratch.run("echo \"i=$[\"1]\"]\"\necho survived\n");
    assert_eq!(code, 0, "continue: exit code (stderr: {stderr})");
    assert_eq!(stdout, "survived\n", "continue: stdout (stderr: {stderr})");
    assert!(
        stderr.contains("bad substitution: no closing `]' in i=$["),
        "continue: stderr {stderr:?}"
    );
}

/// A heredoc body names the whole body string (the string handed to
/// that expansion level), and the command is discarded.
#[test]
fn heredoc_failure_names_whole_body() {
    let scratch = Scratch::new("heredoc");
    let (stdout, stderr, code) =
        scratch.run("cat <<EOF\nx=$[1\ny=$[2]\nEOF\necho after\n");
    assert_eq!(code, 0, "heredoc: exit code (stderr: {stderr})");
    assert_eq!(stdout, "after\n", "heredoc: stdout (stderr: {stderr})");
    assert!(
        stderr.contains("bad substitution: no closing `]' in x=$[1\ny=$[2]"),
        "heredoc: stderr {stderr:?}"
    );
}

/// Neighbour shapes that must keep working: the quote unit INSIDE the
/// span is fine when it is single-quoted, escaped, or the span sits
/// outside double quotes; a plain contained span evaluates.
#[test]
fn valid_neighbour_shapes_still_expand() {
    assert_script("escaped", "echo \"i=$[\\\"1\\\"]\"\n", "i=1\n");
    assert_script("unquoted", "echo $[\"1\"]\n", "1\n");
    assert_script("plain", "echo \"v=$[1+2]\"\n", "v=3\n");
    assert_script("backquote", "echo \"t=$[`echo 4`]\"\n", "t=4\n");
}

/// The single-quoted body extracts whole, then fails as an arithmetic
/// syntax error (a different verdict than the missing `]`).
#[test]
fn single_quoted_body_extracts_then_arith_error() {
    let scratch = Scratch::new("squote");
    let (stdout, stderr, code) = scratch.run("echo \"i=$['1']\"\n");
    assert_eq!(code, 1, "squote: exit code (stderr: {stderr})");
    assert_eq!(stdout, "", "squote: stdout (stderr: {stderr})");
    assert!(
        stderr.contains("arithmetic syntax error"),
        "squote: stderr {stderr:?}"
    );
}
