//! rubash#3833 — reserved words after pending-inversion `!` (and `time`
//! prefixes) run as command words instead of syntax errors (siblings of #3648).
//!
//! GNU model (bash 5.3.20):
//! - `! fi`, `! then`, `! done`, `! esac`, `! do`, `! else`, `! elif` ->
//!   `syntax error near unexpected token `<kw>'`, rc 2.
//! - `! time }`, `time ! }`, `! time fi`, `time ! fi`, `time }`, `time fi`,
//!   `time -p ! }` -> `syntax error near unexpected token `<kw>'`, rc 2 (no
//!   `time` output).
//! - Already-correct shapes preserved: `! }` (rc 2), `! )` (rc 2), `! {`
//!   (EOF rc 2), `! ]` (`]: command not found`, rc 0), `! (echo hi)`
//!   (runs, rc 1), `x=1 ! }` (`!: command not found`, rc 127).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash3833-{tag}-{n}-{}", std::process::id()));
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

fn assert_reserved_error(script: &str, token: &str, line: usize) {
    let (stdout, stderr, rc) = run_c(script);
    assert_eq!(stdout, "", "script {script:?} stderr: {stderr}");
    assert_eq!(rc, 2, "script {script:?} stderr: {stderr}");
    assert_echo(
        &stderr,
        line,
        &format!("syntax error near unexpected token `{token}'"),
        script.lines().nth(line - 1).expect("offending line"),
    );
}

#[test]
fn bang_followed_by_reserved_words_are_syntax_errors() {
    // `in` belongs here, not with the non-reserved shapes below: GNU
    // bash 5.3.20 rejects it at a command position after `!` (and bare)
    // with rc 2 (`syntax error near unexpected token `in''), because
    // the command grammar has no production starting with the IN
    // token. Verified against the oracle 2026-11-18; the earlier
    // placement of `! in` among the unchanged shapes encoded the
    // pre-#3901 product behaviour, which was itself the #3628 defect.
    for kw in ["fi", "then", "done", "esac", "do", "else", "elif", "in"] {
        assert_reserved_error(&format!("! {kw}"), kw, 1);
    }
}

#[test]
fn double_bang_followed_by_reserved_word_is_syntax_error() {
    assert_reserved_error("! ! fi", "fi", 1);
}

#[test]
fn time_and_bang_combinations_before_reserved_words_are_syntax_errors() {
    for (script, token) in [
        ("! time }", "}"),
        ("time ! }", "}"),
        ("time }", "}"),
        ("time -p }", "}"),
        ("time -p ! }", "}"),
        ("time time }", "}"),
        ("! time fi", "fi"),
        ("time ! fi", "fi"),
        ("time fi", "fi"),
        ("time -- fi", "fi"),
    ] {
        assert_reserved_error(script, token, 1);
    }
}

#[test]
fn bang_reserved_word_suppresses_following_and_same_line_commands() {
    assert_reserved_error("! fi; echo hi", "fi", 1);
    assert_reserved_error("echo ok; ! fi", "fi", 1);
}

#[test]
fn multiline_bang_reserved_word_echoes_offending_line_only() {
    let (stdout, stderr, rc) = run_c("echo a\n! fi\necho b");
    assert_eq!(stdout, "a\n", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    assert_echo(
        &stderr,
        2,
        "syntax error near unexpected token `fi'",
        "! fi",
    );
}

#[test]
fn script_file_bang_reserved_word_is_syntax_error() {
    let (stdout, stderr, rc) = run_file("! fi\n");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 2, "stderr: {stderr}");
    assert!(
        lines[0].ends_with("probe.sh: line 1: syntax error near unexpected token `fi'"),
        "diagnostic line: {stderr}"
    );
    assert!(
        lines[1].ends_with("probe.sh: line 1: `! fi'"),
        "echo line: {stderr}"
    );
}

#[test]
fn bang_reserved_word_inside_compound_groups_is_syntax_error() {
    assert_reserved_error("{ ! fi; }", "fi", 1);
    assert_reserved_error("( ! fi )", "fi", 1);
}

#[test]
fn already_correct_and_non_reserved_shapes_stay_unchanged() {
    assert_reserved_error("! }", "}", 1);
    assert_reserved_error("! )", ")", 1);

    let (stdout, stderr, rc) = run_c("! {");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    assert!(
        stderr.contains("unexpected end of file"),
        "unexpected EOF stderr: {stderr}"
    );

    let (stdout, stderr, rc) = run_c("! ]");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 0, "stderr: {stderr}");
    assert!(
        stderr.contains("]: command not found") || stderr.contains("command not found"),
        "expected command-not-found for `! ]`, got: {stderr}"
    );

    let (stdout, stderr, rc) = run_c("! (echo hi)");
    assert_eq!(stdout, "hi\n", "stderr: {stderr}");
    assert_eq!(rc, 1, "stderr: {stderr}");

    let (stdout, stderr, rc) = run_c("x=1 ! }");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 127, "stderr: {stderr}");
    assert!(
        stderr.contains("!: command not found") || stderr.contains("command not found"),
        "expected command-not-found for `x=1 ! }}`, got: {stderr}"
    );

    // `time` after `|` is an ordinary command word, not `timespec`.
    let (_stdout, stderr, rc) = run_c("echo hi | time }");
    assert_ne!(rc, 2, "stderr: {stderr}");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");

    // `-p` after `!` is the timed command's name, not `TIMEOPT`.
    let (_stdout, stderr, rc) = run_c("time ! -p }");
    assert_eq!(rc, 0, "stderr: {stderr}");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
}
