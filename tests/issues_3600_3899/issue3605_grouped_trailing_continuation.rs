//! Issue rubash#3605 (class B, run-parser): a script whose FINAL physical
//! line ends in `\<newline>` must collapse the pair as a continuation
//! (parse.y:5379 `peek_char == '\n'` -> `goto next_character`) — GNU
//! prints `AAA` for `echo AAA\<newline>` at end of input, and keeps the
//! literal backslash only for `\<EOF>` (no trailing newline).
//!
//! The grouped reader (read_next_source_group) strips every raw line's
//! `\n` into per-line `text` entries, and run_history_group re-joined
//! them with `\n` separators — dropping the last line's newline byte.
//! A script whose last line is `... echo AAA\<newline>` then re-parsed
//! as `... echo AAA\<EOF>` and printed the literal `AAA\`. The gather
//! needs the grouped driver to trigger: a plain single-command file
//! takes the whole-source fast path and never re-joins lines.
#![cfg(unix)]

use std::process::Command;

fn run(script_bytes: &[u8]) -> (String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3605-cont-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir fixture");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script_bytes).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// parser1.sub's tail: `eval "array=(foo bar)" ; echo AAA\<newline>` —
/// the `eval` marker routes the script through the grouped driver
/// (script_driver.rs script_runs_foreign_text), where the dropped
/// trailing newline turned GNU's collapsed continuation into a literal
/// backslash. GNU prints `AAA`.
#[test]
fn issue3605_last_line_backslash_newline_collapses() {
    let (stdout, stderr) = run(b"eval \"array=(foo bar)\" ; echo AAA\\\n");
    assert_eq!(stdout, "AAA\n", "stderr: {stderr}");
}

/// Same shape without `eval`: the whole-source fast path already kept
/// the byte — pin that the grouped and fast paths now agree.
#[test]
fn issue3605_fast_path_last_line_backslash_newline_collapses() {
    let (stdout, stderr) = run(b"true ; echo AAA\\\n");
    assert_eq!(stdout, "AAA\n", "stderr: {stderr}");
}

/// `\<EOF>` (no trailing newline) stays a literal backslash — GNU
/// prints `AAA\` — so the fix must key on the input's real newline,
/// not blindly fold a trailing backslash.
#[test]
fn issue3605_backslash_at_eof_stays_literal() {
    let (stdout, stderr) = run(b"eval \"true\" ; echo AAA\\");
    assert_eq!(stdout, "AAA\\\n", "stderr: {stderr}");
}
