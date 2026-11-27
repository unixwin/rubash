//! Issue unixwin/rubash#3904 (A29): a non-WORD token at a compound-
//! assignment list's own level is a yyerror token error, not an EOF
//! diagnostic.
//!
//! GNU anchor: parse_compound_assignment reads each element with
//! read_token (parse.y:7141-7159); a token that is not a WORD or
//! ASSIGNMENT_WORD goes to yyerror, and report_syntax_error
//! (parse.y:6833-6865) names it via error_token_from_token
//! (parse.y:6783-6830) — a bare `(` names itself, a separator or pipe
//! names itself (`;`, `;;`, `;&`, `;;&`, `&`, `&&`, `|`, `||`, `|&`),
//! a redirect names the operator (`>`, `>>`, `>|`, `>&`, `<`, `<>`,
//! `<&`) or, for an fd redirect, its IO_NUMBER digits (`2`). When the
//! list sits inside a command or process substitution, the nested
//! parse set shell_eof_token to `)' (parse.y:4727), so the report
//! appends ` while looking for matching `)'' (parse.y:6858-6859); a
//! plain subshell enclosure does not. The offending line is echoed
//! for non-interactive shells (parse.y:6863-6865) — except under live
//! errexit, where parser_error exits first (error.c:324-327). Status
//! is the token loop's set_exit_status(EXECUTION_FAILURE)
//! (parse.y:7167-7169): 1, or 2 under live errexit.
//!
//! The rubash defect: the script/file unclosed-input route had no arm
//! for this class, so these shapes fell to the generic close-char
//! branch and reported `unexpected EOF while looking for matching
//! `)'' (the list's EOF wording) instead of the token error. The fix
//! classifies the first metachar token at the list's own level in the
//! lexer shape scan (UnclosedComsubShape::CompassignTokenError, held
//! until EOF while its frame is still open) and renders it in the
//! script driver next to the comsub-body token-error arm.
//!
//! Every message/line/code below is measured against GNU bash
//! 5.3.20 (oracle matrix p/q/r/v/w series in the lane's probe dir).

use std::process::{Command, Stdio};

fn run_file(script: &str, tag: &str) -> (String, String, i32) {
    let path = std::env::temp_dir().join(format!("i3904_tok_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
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
        out.status.code().unwrap_or(-1),
    )
}

/// The bare-`(` family: the token error names `(` at the token's line,
/// echoes the offending line, and exits 1 — never the EOF wording.
#[test]
fn compassign_bare_paren_token_error() {
    // (tag, script, expected report line, expected offending line)
    for (tag, script, line, offending) in [
        ("basic", "v=(a (echo", 1usize, "v=(a (echo"),
        ("dparen", "v=((echo", 1, "v=((echo"),
        ("after_word", "v=(a (echo x", 1, "v=(a (echo x"),
        ("space_first", "v=( (echo", 1, "v=( (echo"),
        ("eol", "v=(a (", 1, "v=(a ("),
        ("line_two", "v=(a\n(echo", 2, "(echo"),
        ("nested_assign", "v=(a=(b[1", 1, "v=(a=(b[1"),
        ("subshell", "(v=(a (echo", 1, "(v=(a (echo"),
    ] {
        let (_stdout, stderr, got) = run_file(script, tag);
        assert_eq!(got, 1, "{tag}: {script:?} -> {stderr:?}");
        let needle = format!("line {line}: syntax error near unexpected token `('");
        assert!(stderr.contains(&needle), "{tag}: {stderr:?}");
        assert!(
            stderr.contains(&format!("`{offending}'")),
            "{tag}: {stderr:?}"
        );
        assert!(!stderr.contains("unexpected EOF"), "{tag}: {stderr:?}");
        assert!(
            !stderr.contains("while looking for matching"),
            "{tag}: {stderr:?}"
        );
    }
}

/// Metachar tokens at the list's own level name themselves — and the
/// FIRST bad token wins (`v=(a; (echo` names `;`, not the later `(`).
#[test]
fn compassign_metachar_token_names() {
    // (tag, script, expected token)
    for (tag, script, token) in [
        ("semi_preempts_paren", "v=(a; (echo", ";"),
        ("semi_subscript", "v=(a; b[1", ";"),
        ("pipe_subscript", "v=(a | b[1", "|"),
        ("amp", "v=(a & b", "&"),
        ("amp_amp", "v=(a && b", "&&"),
        ("pipe_pipe", "v=(a || b", "||"),
        ("semi_semi", "v=(a;;", ";;"),
        ("semi_amp", "v=(a ;& b", ";&"),
        ("semi_semi_amp", "v=(a ;;& b", ";;&"),
        ("pipe_amp", "v=(a |& b", "|&"),
        ("gt", "v=(a >x", ">"),
        ("gt_glued", "v=(a>x", ">"),
        ("gt_gt", "v=(a >>x", ">>"),
        ("lt_gt", "v=(a <>x", "<>"),
        ("gt_amp", "v=(a >&x", ">&"),
        ("io_number", "v=(a 2>x", "2"),
        ("io_number_two_digit", "v=(a 22>x", "22"),
        ("word_tail_digits", "v=(a x2>x", ">"),
    ] {
        let (_stdout, stderr, got) = run_file(script, tag);
        assert_eq!(got, 1, "{tag}: {script:?} -> {stderr:?}");
        let needle = format!("line 1: syntax error near unexpected token `{token}'");
        assert!(stderr.contains(&needle), "{tag}: {stderr:?}");
        assert!(!stderr.contains("unexpected EOF"), "{tag}: {stderr:?}");
    }
}

/// Inside a command or process substitution the report carries the
/// matching suffix (shell_eof_token `)'), in file mode and in eval.
#[test]
fn compassign_token_error_matching_suffix() {
    for (tag, script) in [
        ("comsub_paren", "echo $(v=(a (echo"),
        ("comsub_semi", "echo $(v=(a; b[1"),
        ("procsub", "cat <(v=(a (echo)"),
    ] {
        let (_stdout, stderr, got) = run_file(script, tag);
        assert_eq!(got, 1, "{tag}: {script:?} -> {stderr:?}");
        assert!(
            stderr
                .contains("syntax error near unexpected token `(' while looking for matching `)'")
                || stderr.contains(
                    "syntax error near unexpected token `;' while looking for matching `)'"
                ),
            "{tag}: {stderr:?}"
        );
    }

    let (_stdout, stderr, got) = run_file("eval 'echo $(v=(a (echo'", "eval_comsub");
    assert_eq!(got, 1, "eval_comsub: {stderr:?}");
    assert!(
        stderr.contains("syntax error near unexpected token `(' while looking for matching `)'"),
        "eval_comsub: {stderr:?}"
    );
}

/// Under live errexit the status is 2 and the offending line is NOT
/// echoed (parser_error exits before print_offending_line).
#[test]
fn compassign_token_error_errexit() {
    let (_stdout, stderr, got) = run_file("set -e\nv=(a (echo", "errexit_paren");
    assert_eq!(got, 2, "errexit_paren: {stderr:?}");
    assert!(
        stderr.contains("line 2: syntax error near unexpected token `('"),
        "errexit_paren: {stderr:?}"
    );
    assert!(
        !stderr.contains("`v=(a (echo'"),
        "errexit_paren: {stderr:?}"
    );

    let (_stdout, stderr, got) = run_file("set -e\necho $(v=(a (echo", "errexit_comsub");
    assert_eq!(got, 2, "errexit_comsub: {stderr:?}");
    assert!(
        stderr.contains("while looking for matching `)'"),
        "errexit_comsub: {stderr:?}"
    );
    assert!(
        !stderr.contains("`echo $(v=(a (echo'"),
        "errexit_comsub: {stderr:?}"
    );
}

/// Complete commands before the failing line already ran.
#[test]
fn compassign_token_error_prefix_runs() {
    let (stdout, stderr, got) = run_file("echo ok\nv=(a (echo", "prefix");
    assert_eq!(got, 1, "prefix: {stderr:?}");
    assert_eq!(stdout, "ok\n", "prefix: stdout {stdout:?}");
    assert!(
        stderr.contains("line 2: syntax error near unexpected token `('"),
        "prefix: {stderr:?}"
    );
}

/// A13 controls: the subscript shapes keep the list's `)` EOF report —
/// the token-error arm must not claim them.
#[test]
fn compassign_subscript_controls_stay_eof() {
    for (tag, script) in [("a13_mid", "v=(a[1"), ("a13_nl_mid", "v=(a\nb[1")] {
        let (_stdout, stderr, got) = run_file(script, tag);
        assert_eq!(got, 1, "{tag}: {script:?} -> {stderr:?}");
        assert!(
            stderr.contains("line 1: unexpected EOF while looking for matching `)'"),
            "{tag}: {stderr:?}"
        );
    }
}
