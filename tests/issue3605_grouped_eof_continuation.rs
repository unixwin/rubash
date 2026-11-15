//! rubash#3605 / run-parser parser1.sub: the grouped reader's exec text
//! re-joins the group's TRIMMED line texts with '\n', so a script-final
//! `\<newline>` continuation reached the lexer as a backslash at bare
//! EOF — kept literal (parse.y:2739-2743) — instead of the elided pair
//! GNU's shell_getc produces (parse.y:2842: the `\<newline>` is removed
//! and the read continues past the physical line end). The group's exec
//! text now carries the input's real trailing terminator.
//!
//! Only the grouped driver was affected: `eval`/`source`/`.` route a
//! script through run_script_with_history (script_runs_foreign_text —
//! builtins/evalstring.c:359's incremental parse loop), while a plain
//! single-line script takes the whole-file fast path that already
//! elided the pair.
//!
//! Expectations verified against GNU bash 5.3.20 script-file probes.

use std::process::{Command, Stdio};

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-3605-eofcont-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn eval_script_trailing_backslash_newline_elides() {
    // parser1.sub's exact shape (grouped driver via `eval`).
    let (stdout, stderr, status) = rubash_script("eval \"array=(foo bar)\" ; echo AAA\\\n");
    assert_eq!(stdout, "AAA\n", "stderr: {stderr}");
    assert_eq!(status, Some(0));
}

#[test]
fn foreign_text_line_before_trailing_continuation() {
    // The failing shape was a grouped multi-line script whose LAST line
    // ended `\<newline>`: the group join dropped that terminator.
    let (stdout, stderr, status) = rubash_script("eval \"echo x\"\necho AAA\\\n");
    assert_eq!(stdout, "x\nAAA\n", "stderr: {stderr}");
    assert_eq!(status, Some(0));
}

#[test]
fn bare_eof_backslash_stays_literal_in_grouped_driver() {
    // parse.y:2739-2743: a backslash at bare EOF is NOT removed — only
    // the `\<newline>` pair elides. The terminator flag must not invent
    // a newline the input did not have.
    let (stdout, stderr, status) = rubash_script("eval \"echo x\"\necho AAA\\");
    assert_eq!(stdout, "x\nAAA\\\n", "stderr: {stderr}");
    assert_eq!(status, Some(0));
}

#[test]
fn mid_group_continuation_still_joins() {
    // Regression pin: `\<newline>` inside the group already elided via
    // the re-joined newline separator.
    let (stdout, stderr, status) = rubash_script("eval \"echo x\"\necho AAA\\\necho BBB\n");
    assert_eq!(stdout, "x\nAAAecho BBB\n", "stderr: {stderr}");
    assert_eq!(status, Some(0));
}
