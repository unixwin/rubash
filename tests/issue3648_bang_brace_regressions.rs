//! rubash#3648 — `! } { }` runs `}` as a command instead of a syntax error.
//!
//! GNU model (bash 5.2.21, verified by oracle probe on the box):
//! - `! } { }` -> `bash: -c: line 1: syntax error near unexpected token `}'`
//!   + `bash: -c: line 1: `! } { }'`, rc 2.
//! - The same holds for `! }`, `{ ! } }`, `! } ; echo hi` (nothing after the
//!   error runs) and `echo ok; ! }` (the `ok` is suppressed: GNU parses the
//!   whole line before executing any of it).
//! - `! ! }` is also a syntax error (the second `!` toggles the inversion
//!   back off, so the bare-`}` arm already caught it).
//! - Legal after `!`: `{` (brace group), `(` (subshell), `time`, another `!`.
//!
//! Root cause: the `}` arm in `src/parser/token_actions.rs` gated on
//! `command_is_empty` only. The `!` arm had already toggled `inverted`, so
//! the command was no longer empty, the `}` fell through to word handling,
//! ran as a command name, and the `!` flipped the not-found failure to
//! rc 0. The fix admits a pending inversion (`command_is_pending_inversion`)
//! in the `}` arm — the same predicate `command_allows_compound_start`
//! already uses for reserved positions.
//!
//! Out of scope (verified divergent on the base, left for follow-ups):
//! `! fi` / `! then` / `! done` / `! esac` / `! do` / `! else` / `! elif`
//! run the word as a command (rc 0, GNU rc 2) — the sibling reserved-word
//! arm has the same empty-only gate; `! time }` / `time ! }` likewise run
//! `}` as a command — the `}` there follows a `time` prefix, not a pending
//! inversion.
//!
//! The `-c` assertions match on line suffixes, not the full stderr text:
//! the diagnostic prefix carries argv[0], which is the test binary path
//! under `cargo test` and `bash` under GNU (see the pre-existing stale
//! `bash:` expectations in `tests/issue360_parser_overlax_regressions.rs`).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Each test run gets its own tempdir: the tests run in parallel and
/// sharing one dir per tag lets one test's remove_dir_all delete another
/// test's child cwd mid-run (spurious `shell-init: getcwd` errors).
static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash3648-{tag}-{n}-{}", std::process::id()));
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

fn assert_brace_error(script: &str, line: usize) {
    let (stdout, stderr, rc) = run_c(script);
    assert_eq!(stdout, "", "script {script:?} stderr: {stderr}");
    assert_eq!(rc, 2, "script {script:?} stderr: {stderr}");
    assert_echo(
        &stderr,
        line,
        "syntax error near unexpected token `}'",
        script.lines().nth(line - 1).expect("offending line"),
    );
}

#[test]
fn seed_bang_brace_is_syntax_error() {
    assert_brace_error("! } { }", 1);
}

#[test]
fn bare_bang_brace_is_syntax_error() {
    assert_brace_error("! }", 1);
}

#[test]
fn bang_brace_inside_group_is_syntax_error() {
    assert_brace_error("{ ! } }", 1);
}

#[test]
fn bang_brace_suppresses_following_command() {
    assert_brace_error("! } ; echo hi", 1);
}

#[test]
fn bang_brace_suppresses_same_line_command() {
    assert_brace_error("echo ok; ! }", 1);
}

#[test]
fn multiline_bang_brace_echoes_offending_line_only() {
    let (stdout, stderr, rc) = run_c("echo a\n! }\necho b");
    assert_eq!(stdout, "a\n", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    assert_echo(&stderr, 2, "syntax error near unexpected token `}'", "! }");
}

#[test]
fn double_bang_brace_is_syntax_error() {
    assert_brace_error("! ! }", 1);
}

#[test]
fn script_file_bang_brace_is_syntax_error() {
    let (stdout, stderr, rc) = run_file("! } { }\n");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(rc, 2, "stderr: {stderr}");
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 2, "stderr: {stderr}");
    assert!(
        lines[0].ends_with("probe.sh: line 1: syntax error near unexpected token `}'"),
        "diagnostic line: {stderr}"
    );
    assert!(
        lines[1].ends_with("probe.sh: line 1: `! } { }'"),
        "echo line: {stderr}"
    );
}

#[test]
fn legal_bang_positions_stay_legal() {
    for (script, stdout, rc) in [
        ("! true", "", 1),
        ("! ! true", "", 0),
        ("! { echo hi; }", "hi\n", 1),
        ("{ ! true; }", "", 1),
        ("{ ! false; }", "", 0),
        ("! (true)", "", 1),
    ] {
        let (got_out, stderr, got) = run_c(script);
        assert_eq!(got_out, stdout, "script {script:?} stderr: {stderr}");
        assert_eq!(stderr, "", "script {script:?}");
        assert_eq!(got, rc, "script {script:?} stderr: {stderr}");
    }
    // `time` prints its timing report to stderr (both shells); only the
    // exit code is asserted here.
    let (got_out, stderr, got) = run_c("! time");
    assert_eq!(got_out, "", "stderr: {stderr}");
    assert!(stderr.contains("real"), "stderr: {stderr}");
    assert_eq!(got, 1, "stderr: {stderr}");
}
