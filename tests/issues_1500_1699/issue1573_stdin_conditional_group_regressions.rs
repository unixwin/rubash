//! Issue unixwin/rubash#1573: the stdin group splitter had no
//! `[[ ... ]]` tracking at all — a conditional spanning physical lines
//! was submitted after its first line and reported the EOF shape
//! (`unexpected EOF while looking for `]]'` + the compound-EOF tail)
//! instead of being gathered as one command the way GNU's reader
//! gathers it (parse.y:3654 PST_CONDCMD: the conditional is one
//! command; a newline inside it is a token separator).
//!
//! The completeness test's token-level half now asks the scanner's own
//! PST_CONDCMD state in both of its implementations
//! (GroupScanFeeder::token_level_needs_more and the fresh-scan mirror
//! stdin_source_needs_more_posix via lexer::source_ends_in_cond_command).
//! The regexp-absorbed shape from the issue is pinned in
//! issue1553_regexp_absorbed_close_ws_regressions.rs
//! (issue1553_stdin_form_reads_next_line, activated by this fix).
//!
//! Verified against GNU Bash 5.3.20 (stdin probes).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn issue1573_plain_multiline_conditional_via_stdin() {
    let (stdout, stderr, code) = run_rubash_stdin("[[ a == a\n]]\necho done=$?\n");
    assert_eq!(stdout, "done=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1573_continuation_line_operator_via_stdin() {
    let (stdout, stderr, code) = run_rubash_stdin("[[ a == a\n&& b == b ]]\necho done=$?\n");
    assert_eq!(stdout, "done=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1573_conditional_result_drives_following_commands() {
    let (stdout, stderr, code) =
        run_rubash_stdin("[[ a == z\n]]\necho done=$?\n[[ a == a ]]; echo second=$?\n");
    assert_eq!(stdout, "done=1\nsecond=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1573_unterminated_conditional_at_eof_reports_eof_shape() {
    // Control: at real EOF the group IS submitted unterminated and the
    // EOF diagnostics are GNU's pair (the `]]` want + the compound
    // tail), not silence.
    let (stdout, stderr, code) = run_rubash_stdin("[[ a == a\n");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("unexpected EOF while looking for `]]'"),
        "missing ]]-want diagnostic: {stderr:?}"
    );
    assert!(
        stderr.contains("unexpected end of file from `[[' command on line 1"),
        "missing compound EOF tail: {stderr:?}"
    );
    assert_eq!(code, Some(2));
}
