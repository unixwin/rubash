//! Issue rubash#1840: `echo hi | time ! cat` was a syntax error
//! (`near unexpected token `!'`), because the `| !` guard
//! (rubash#360: BANG may not follow a pipe connector) fired for
//! any `!` while a pipe was pending on the previous stage —
//! including a `!` read after the new stage's command had
//! already begun.
//!
//! GNU: a standalone `!` is the BANG token only in command
//! position — a completely fresh command (nothing consumed: no
//! word, no assignment, no redirection). Once any element of
//! the simple command exists, `!` is an ordinary word: after
//! `time` it is an argument (the stage resolves `time` through
//! PATH, 127), after a redirection or assignment prefix it is
//! the command NAME (`!: command not found`). Directly after
//! `|` the stage IS fresh, BANG is read, and the grammar
//! rejects it (pipeline_command, parse.y:1408-1413) — that
//! error must stay. Verified against GNU bash 5.3.20
//! (self-compiled oracle), script-file mode.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1840-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1840_bang_after_time_word_is_an_argument() {
    // The issue's shape: `!` after the stage's first word is an
    // argument of the external `time` command name — 127, never
    // a syntax error.
    let (stdout, stderr, code) =
        rubash_file("PATH=/nonexistent\necho hi | time ! cat\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=127\n");
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1840_bang_glued_to_time_is_one_word() {
    // `time!` lexes as ONE word (the `!` is not a delimiter), so
    // the command name in the diagnostic carries it.
    let (stdout, stderr, _) =
        rubash_file("PATH=/nonexistent\necho hi | time! cat\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=127\n");
    assert!(
        stderr.contains("time!: command not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1840_bang_glued_word_is_an_argument() {
    // `!cat` is one argument word; `time` is the command name.
    let (stdout, stderr, _) =
        rubash_file("PATH=/nonexistent\necho hi | time !cat\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=127\n");
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
}

#[test]
fn issue1840_bang_after_redirect_prefix_is_the_command_name() {
    // A redirection prefix does not open BANG position either:
    // with the stage otherwise fresh, `!` becomes the command
    // name of the pending simple command.
    let (stdout, stderr, _) = rubash_file("echo hi | >f ! cat\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=127\n");
    assert!(stderr.contains("!: command not found"), "stderr: {stderr}");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
}

#[test]
fn issue1840_bang_after_assignment_prefix_is_the_command_name() {
    let (stdout, stderr, _) = rubash_file("echo hi | x=1 ! cat\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=127\n");
    assert!(stderr.contains("!: command not found"), "stderr: {stderr}");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
}

#[test]
fn issue1840_bang_directly_after_pipe_stays_a_syntax_error() {
    // Control (rubash#360): the stage is fresh after `|`, so
    // `!` IS the BANG token and the grammar rejects it — GNU
    // rc 2, nothing runs.
    let (stdout, stderr, code) = rubash_file("echo hi | ! cat\necho \"S=$?\"\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("syntax error near unexpected token `!'"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1840_bang_at_pipeline_head_still_negates() {
    // Control: BANG prefixing a whole pipeline (with a redirect
    // prefix on the negated command) keeps working.
    let (stdout, _, code) = rubash_file("! >f true\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=1\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1840_time_bang_assignment_at_head_still_times() {
    // Control (parser-time family): `time ! x=1` at a list head
    // is the timespec over a negated assignment-only command.
    let (stdout, stderr, _) = rubash_file("time ! x=1\necho \"S=$? x=$x\"\n");
    assert_eq!(stdout, "S=1 x=1\n");
    assert!(
        stderr.lines().any(|line| line.starts_with("user\t")),
        "time report missing: {stderr}"
    );
}
