//! Issue rubash#1839: a compound command after a pipeline-middle
//! `time` was admitted as a timed compound (`echo hi | time
//! { echo x; }` ran and printed a time report).
//!
//! GNU: the timespec production hangs off pipeline_command
//! (parse.y:1416), a pipeline's later elements derive from
//! `command` alone, and time_command_acceptable (parse.y:3286) has
//! no `|' case — after a pipe connector `time` is an ordinary
//! command NAME, the compound tokens are its argument text, and
//! the reserved word that lands back in command position after
//! the terminating `;' (or the metachar in argument position)
//! is the syntax error the grammar reports, rc 2, with nothing
//! on the line executed. Verified against GNU bash 5.3.20
//! (self-compiled oracle), script-file mode.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1839-{}-{}",
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

/// A mis-admitted timed compound both runs its body and leaves a
/// shell-format time report on stderr; a GNU-faithful syntax error
/// does neither, and no command of the line may have executed.
fn assert_syntax_error(script: &str, offender: &str) {
    let (stdout, stderr, code) = rubash_file(&format!("{script}\necho \"S=$?\"\n"));
    assert_eq!(code, Some(2), "script: {script} stderr: {stderr}");
    assert_eq!(stdout, "", "script: {script} stderr: {stderr}");
    assert!(
        stderr.contains(&format!("syntax error near unexpected token `{offender}'")),
        "script: {script} stderr: {stderr}"
    );
    assert!(
        stderr.contains("case.sh: line 1:"),
        "script: {script} stderr: {stderr}"
    );
    assert!(
        !stderr.lines().any(|line| {
            line.starts_with("real\t")
                || line.starts_with("real ")
                || line.starts_with("user\t")
                || line.starts_with("sys\t")
        }),
        "unexpected time report, script: {script} stderr: {stderr}"
    );
}

#[test]
fn issue1839_pipeline_middle_time_brace_group_is_syntax_error() {
    assert_syntax_error("echo hi | time { echo x; }", "}");
}

#[test]
fn issue1839_pipeline_middle_time_posix_brace_group_is_syntax_error() {
    assert_syntax_error("echo hi | time -p { echo x; }", "}");
}

#[test]
fn issue1839_pipeline_middle_time_nested_time_brace_group_is_syntax_error() {
    assert_syntax_error("echo hi | time time { echo x; }", "}");
}

#[test]
fn issue1839_pipeline_middle_time_pipe_err_brace_group_is_syntax_error() {
    assert_syntax_error("echo hi |& time { echo x; }", "}");
}

#[test]
fn issue1839_pipeline_middle_time_if_is_syntax_error_at_then() {
    assert_syntax_error("echo hi | time if true; then echo x; fi", "then");
}

#[test]
fn issue1839_pipeline_middle_time_for_is_syntax_error_at_do() {
    assert_syntax_error("echo hi | time for i in 1; do echo x; done", "do");
}

#[test]
fn issue1839_pipeline_middle_time_while_is_syntax_error_at_do() {
    assert_syntax_error("echo hi | time while false; do echo x; done", "do");
}

#[test]
fn issue1839_pipeline_middle_time_select_is_syntax_error_at_do() {
    assert_syntax_error(
        "echo hi | time select s in a; do echo $s; break; done",
        "do",
    );
}

#[test]
fn issue1839_pipeline_middle_time_case_is_syntax_error_at_rparen() {
    assert_syntax_error("echo hi | time case v in v) echo y;; esac", ")");
}

#[test]
fn issue1839_pipeline_middle_time_arithmetic_is_syntax_error_at_lparen() {
    assert_syntax_error("echo hi | time ((1))", "(");
}

#[test]
fn issue1839_pipeline_middle_time_function_def_is_syntax_error_at_lparen() {
    assert_syntax_error("echo hi | time f() { echo x; }", "(");
}

#[test]
fn issue1839_pipeline_middle_time_subshell_is_syntax_error_at_word() {
    // GNU names the word after the `(': the shifted `(' expects
    // `)' (function head) or a subshell, and `echo' is neither.
    assert_syntax_error("echo hi | time (echo x)", "echo");
}

#[test]
fn issue1839_pipeline_middle_time_coproc_brace_is_syntax_error() {
    assert_syntax_error("echo hi | time coproc { echo x; }", "}");
}

#[test]
fn issue1839_pipeline_middle_time_newline_after_pipe_is_syntax_error() {
    // A pipeline continues across the newline: the head on the
    // next physical line is still a pipeline-middle word.
    let (stdout, stderr, code) = rubash_file("echo hi |\ntime { echo x; }\necho \"S=$?\"\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert!(
        stderr.contains("syntax error near unexpected token `}'"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("case.sh: line 2:"), "stderr: {stderr}");
}

#[test]
fn issue1839_pipeline_middle_time_conditional_is_a_command_name() {
    // Not a syntax error: `[[` in argument position is a plain
    // word to GNU, so the middle stage resolves `time` through
    // PATH and fails 127 with no timing.
    let (stdout, stderr, code) =
        rubash_file("PATH=/nonexistent\necho hi | time [[ -n x ]]\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=127\n");
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue1839_pipeline_head_time_brace_group_still_times() {
    // Control: the pipeline HEAD keeps the timespec over a
    // compound command (GNU times the group).
    let (stdout, stderr, code) = rubash_file("time { echo x; } | cat\necho \"S=$?\"\n");
    assert_eq!(stdout, "x\nS=0\n");
    assert!(
        stderr.lines().any(|line| line.starts_with("user\t")),
        "head time report missing: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue1839_group_stage_time_still_times_inside_the_group() {
    // Control: a brace group in a later pipeline position starts
    // a fresh command list, whose head `time` IS the keyword.
    let (stdout, _, code) = rubash_file("echo hi | { time echo x; }\necho \"S=$?\"\n");
    assert_eq!(stdout, "x\nS=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1839_pipeline_head_time_posix_brace_group_still_times() {
    // Control: head `time -p` over a brace group (POSIX report).
    let (stdout, stderr, code) = rubash_file("time -p { echo x; }\necho \"S=$?\"\n");
    assert_eq!(stdout, "x\nS=0\n");
    assert!(
        stderr.lines().any(|line| line.starts_with("real ")),
        "posix time report missing: {stderr}"
    );
    assert_eq!(code, Some(0));
}
