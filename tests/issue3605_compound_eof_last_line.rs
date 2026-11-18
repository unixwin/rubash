//! Issue unixwin/rubash#3605 (run-heredoc residual): the unclosed-compound
//! EOF report ("unexpected end of file from `X' command on line N")
//! stamped its report line from token `position` alone. GNU's
//! line_number at EOF counts every physical line the reader consumed
//! (parse.y:2504's per-line bump plus the final exhaustion read), which
//! includes lines the position stamp does not see:
//!
//! - a heredoc body gathered to EOF consumed through
//!   `Token::heredoc_end_line` while its `position` stays at the `<<'
//!   line (it feeds the `at line N' warning);
//! - a trailing `\n\n' / blank / comment tail's line-break separator
//!   ends at `end_position` while its `position` is its first line.
//!
//! `{ cat <<EOF\nx` reported the `{'-EOF at line 2 where GNU reports 3;
//! `{ echo x\n\n` reported 2 where GNU reports 3. `last_line` now folds
//! `position`/`end_position`/`heredoc_end_line` into one
//! last-consumed-line across the unclosed `{`/`(`/keyword EOF arms
//! (unclosed_brace_eof_node, unclosed_paren_eof_node,
//! unclosed_keyword_eof_node, push_unclosed_paren_error).
//!
//! GNU owner: parse.y:6890-6901 yyerror EOF path — the report prints
//! `line_number` once input is exhausted.
//!
//! Expectations verified byte-identical against GNU Bash 5.3.20
//! (2026-11-15).

use std::process::{Command, Stdio};

fn run_rubash_source(source: &str, tag: &str) -> (String, String, Option<i32>) {
    let path =
        std::env::temp_dir().join(format!("issue3605_ceof_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, source).expect("write probe script");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// `{ cat <<EOF\nx`: the unterminated gather consumed physical line 2,
/// so the `{'-EOF report lands at line 3 (GNU), not the `<<' line + 1.
#[test]
fn brace_eof_after_unterminated_heredoc_reports_consumed_line() {
    let (_stdout, stderr, _code) = run_rubash_source("{ cat <<EOF\nx\n", "bracehd");
    assert!(
        stderr.contains("line 3: syntax error: unexpected end of file from `{' command on line 1"),
        "stderr: {stderr}"
    );
}

/// `f() (\ncat <<EOF\nx`: the same fold applies to the `(` EOF arm
/// inside a function body — GNU reports line 4.
#[test]
fn inner_paren_eof_after_unterminated_heredoc_reports_consumed_line() {
    let (_stdout, stderr, _code) = run_rubash_source("f() (\ncat <<EOF\nx\n", "parenhd");
    assert!(
        stderr.contains("line 4: syntax error: unexpected end of file from `(' command on line 1"),
        "stderr: {stderr}"
    );
}

/// `{ echo x\n\n\n\n`: trailing blank lines are consumed input too —
/// the line-break separators' `end_position` must feed the report
/// (GNU line 5, not the first separator's line).
#[test]
fn brace_eof_counts_trailing_blank_lines() {
    let (_stdout, stderr, _code) = run_rubash_source("{ echo x\n\n\n\n", "blanks");
    assert!(
        stderr.contains("line 5: syntax error: unexpected end of file from `{' command on line 1"),
        "stderr: {stderr}"
    );
}

/// `{ echo x\n# note`: a trailing comment line counts as consumed —
/// GNU reports line 3.
#[test]
fn brace_eof_counts_trailing_comment_line() {
    let (_stdout, stderr, _code) = run_rubash_source("{ echo x\n# note", "comment");
    assert!(
        stderr.contains("line 3: syntax error: unexpected end of file from `{' command on line 1"),
        "stderr: {stderr}"
    );
}
