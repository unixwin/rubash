//! rubash#2265 — `<<-` (deblank) here-documents on a NUMBERED fd
//! (`3<<-EOF`) lost their tab-stripping entirely: body lines kept
//! their leading tabs and a tab-indented terminator line never
//! matched, so the gather ran to end-of-input and swallowed the
//! following script lines into the body. This is the root cause of
//! the six git t0000 test-lib self-check divergences (#7
//! --immediate, #34/#35 --run range errors, #36/#37/#40
//! --invert-exit-code): every one of them calls
//! `check_sub_test_lib_test_err` with `<<-EOF_OUT 3<<-EOF_ERR` on a
//! function call, so the expected-stderr stream read from fd 3 was
//! the raw remainder of the input instead of the heredoc body,
//! while the sub-scripts' own stdout/stderr matched GNU exactly.
//!
//! GNU: parse.y:643-661 produces r_deblank_reading_until for all
//! three `<<-` grammar forms — bare, `NUMBER LESS_LESS_MINUS WORD`,
//! and `REDIR_WORD LESS_LESS_MINUS WORD` — and make_cmd.c:527
//! (make_here_document) derives kill_leading from that instruction
//! alone, never from the fd prefix. Rubash's lexer merges the fd
//! digits and the operator into ONE token whose value is the whole
//! slice (`3<<-`, lexer/number_redirect.rs), and
//! lexer/heredoc.rs::heredoc_delimiters decided strip_tabs by exact
//! equality with `"<<-"`, which the numbered form can never
//! satisfy; the parser-side flag (redirections.rs heredoc_redirect,
//! `operator.ends_with("<<-")`) was already correct. The fix makes
//! the lexer decision ends_with-based, matching the parser.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (script-file
//! probes p1-p13/q1/q4/q6 in the cluster workdir).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn func_fd3_dash_heredoc_strips_tabs_and_terminates() {
    let (stdout, stderr, code) = rubash(
        "show3() { echo \"--fd3--\"; cat <&3; }\n\
         show3 3<<-EOF_ERR\n\
         \tERRLINE\n\
         \tEOF_ERR\n\
         echo \"--done--\"",
    );
    assert_eq!(stdout, "--fd3--\nERRLINE\n--done--\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn func_two_dash_heredocs_stdin_and_fd3() {
    let (stdout, _, code) = rubash(
        "show() { echo \"--stdin--\"; cat; echo \"--fd3--\"; cat <&3; }\n\
         show <<-EOF_OUT 3<<-EOF_ERR\n\
         \tOUTLINE\n\
         \tEOF_OUT\n\
         \tERRLINE\n\
         \tEOF_ERR\n\
         echo \"--done--\"",
    );
    assert_eq!(stdout, "--stdin--\nOUTLINE\n--fd3--\nERRLINE\n--done--\n");
    assert_eq!(code, Some(0));
}

#[test]
fn func_fd3_dash_heredoc_empty_body_t0000_check_shape() {
    // The exact git test-lib check shape: fd-3 heredoc with an EMPTY
    // body must deliver an empty stream, not the terminator line and
    // the rest of the input.
    let (stdout, _, code) = rubash(
        "chk() { echo \"--stdin--\"; cat; echo \"--fd3--\"; cat <&3; echo \"--end--\"; }\n\
         chk <<-EOF_OUT 3<<-EOF_ERR\n\
         \tOUTLINE\n\
         \tEOF_OUT\n\
         \tEOF_ERR\n\
         echo \"--done--\"",
    );
    assert_eq!(stdout, "--stdin--\nOUTLINE\n--fd3--\n--end--\n--done--\n");
    assert_eq!(code, Some(0));
}

#[test]
fn func_fd3_nodash_heredoc_unchanged() {
    let (stdout, _, code) = rubash(
        "show3() { echo \"--fd3--\"; cat <&3; }\n\
         show3 3<<EOF_ERR\n\
         ERRLINE\n\
         EOF_ERR\n\
         echo \"--done--\"",
    );
    assert_eq!(stdout, "--fd3--\nERRLINE\n--done--\n");
    assert_eq!(code, Some(0));
}

#[test]
fn func_fd3_dash_heredoc_bare_terminator_still_strips_body() {
    let (stdout, _, code) = rubash(
        "show3() { echo \"--fd3--\"; cat <&3; }\n\
         show3 3<<-EOF_ERR\n\
         \tERRLINE\n\
         EOF_ERR\n\
         echo \"--done--\"",
    );
    assert_eq!(stdout, "--fd3--\nERRLINE\n--done--\n");
    assert_eq!(code, Some(0));
}

#[test]
fn func_fd4_dash_heredoc_other_fd_number() {
    let (stdout, _, code) = rubash(
        "show4() { echo \"--fd4--\"; cat <&4; }\n\
         show4 4<<-EOF4\n\
         \tLINE4\n\
         \tEOF4\n\
         echo \"--done--\"",
    );
    assert_eq!(stdout, "--fd4--\nLINE4\n--done--\n");
    assert_eq!(code, Some(0));
}

#[test]
fn func_fd3_first_stdin_second_order_swap() {
    let (stdout, stderr, code) = rubash(
        "show() { echo \"--stdin--\"; cat; echo \"--fd3--\"; cat <&3; }\n\
         show 3<<-EOF_ERR <<-EOF_OUT\n\
         \tERRLINE\n\
         \tEOF_ERR\n\
         \tOUTLINE\n\
         \tEOF_OUT\n\
         echo \"--done--\"",
    );
    assert_eq!(stdout, "--stdin--\nOUTLINE\n--fd3--\nERRLINE\n--done--\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn brace_group_fd3_dash_heredoc() {
    let (stdout, _, code) = rubash(
        "{ echo \"--stdin--\"; cat; echo \"--fd3--\"; cat <&3; } <<-EOF_OUT 3<<-EOF_ERR\n\
         \tOUTLINE\n\
         \tEOF_OUT\n\
         \tERRLINE\n\
         \tEOF_ERR\n\
         echo \"--done--\"",
    );
    assert_eq!(stdout, "--stdin--\nOUTLINE\n--fd3--\nERRLINE\n--done--\n");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn subshell_fd3_dash_heredoc() {
    let (stdout, _, code) = rubash(
        "( echo \"--fd3--\"; cat <&3 ) 3<<-EOF_ERR\n\
         \tERRLINE\n\
         \tEOF_ERR\n\
         echo \"--done--\"",
    );
    assert_eq!(stdout, "--fd3--\nERRLINE\n--done--\n");
    assert_eq!(code, Some(0));
}

#[test]
fn read_builtin_fd3_dash_heredoc() {
    let (stdout, _, code) = rubash(
        "read -u 3 line 3<<-EOF\n\
         \tTABLINE\n\
         \tEOF\n\
         echo \"got:[$line]\"",
    );
    assert_eq!(stdout, "got:[TABLINE]\n");
    assert_eq!(code, Some(0));
}

#[test]
fn while_loop_fd3_dash_heredoc() {
    let (stdout, _, code) = rubash(
        "while IFS= read -u 3 -r l; do echo \"[$l]\"; done 3<<-EOF\n\
         \tLOOPLINE\n\
         \tEOF\n\
         echo \"--done--\"",
    );
    assert_eq!(stdout, "[LOOPLINE]\n--done--\n");
    assert_eq!(code, Some(0));
}
