//! Issue rubash#305 regressions (wt12/heredocfix lane).
//!
//! `echo x <<EOF>#comment` — the `#` tail after the heredoc delimiter word
//! was treated as heredoc data: rubash parsed `>` as a redirection whose
//! FILENAME became the first heredoc body line (`body line: Invalid
//! argument`, rc=1, script continued). GNU: the delimiter WORD ends at the
//! `>` metacharacter (parse.y:5305 read_token_word, shellbreak at 5688),
//! `>` is a GREATER operator token (parse.y:3667 shellmeta), the `#` tail
//! is a comment at token boundary (parse.y:3630-3643), and the '\n' is
//! read only AFTER gather_here_documents consumed the bodies (parse.y
//! 3648-3654, gather at 3651) — so yacc sees GREATER followed by NEWLINE:
//! `syntax error near unexpected token `newline'' (report_syntax_error
//! parse.y:6833) REPORTED at the post-gathering line (each body line
//! advanced line_number, make_cmd.c:580) with print_offending_line
//! (parse.y:6814) echoing the HEADER line. Nothing on the logical line
//! runs (`echo before; echo x <<EOF>#c' runs NOTHING — the
//! newline-terminated list is one parse unit).
//!
//! Root causes fixed:
//! 1. `is_redirect_target_token` (parser/redirections.rs) admitted
//!    TokenKind::HereDocBody as a redirection operand — GNU never has a
//!    body token in the grammar (gather happens inside read_token), so the
//!    body was eaten as the `>`'s filename.
//! 2. `missing_redirect_target_node` (parser/token_actions.rs) had no
//!    HereDocBody arm and attached no source-line echo; it now names
//!    `newline' at the post-gathering line carried by the new
//!    Token::heredoc_end_line, echoes the header line via the source text
//!    (token raws lose comments), and carries EOF-gather warnings that
//!    GNU prints BEFORE the error (make_cmd.c:626).
//! 3. The compound-follow check (parser/parse_loop.rs) routed a redirect
//!    operator the trailing collector stopped at to the generic
//!    "unexpected token" error (`near `>''), masking the real
//!    missing-operand diagnosis for `{ ...; } <<EOF>#c',
//!    `while ...; done <<EOF>#c', `if ...; fi <<EOF>#c', `(:) <<EOF>#c'.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28; the 40-case matrix
//! lives under target/issue-suites/results/issue305/).

use std::process::Command;

/// Run a script FILE in its own scratch directory (relative name `case.sh`
/// keeps the diagnostic prefix stable) and return (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i305-{}-{}",
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

/// The core repro: `#` tail after `<<EOF>`; error names `newline' at the
/// CLOSING-delimiter line (body = 2 lines), echo shows the header line,
/// exit 2, nothing runs.
#[test]
fn heredoc_word_hash_tail_after_redirect_is_newline_error() {
    let (stdout, stderr, code) = rubash_file("echo x <<EOF>#comment\nbody line\nEOF\necho rc=$?\n");
    assert!(stdout.is_empty(), "stdout not empty: {stdout}");
    assert_eq!(
        stderr,
        "case.sh: line 3: syntax error near unexpected token `newline'\n\
         case.sh: line 3: `echo x <<EOF>#comment'\n"
    );
    assert_eq!(code, Some(2));
}

/// Empty body: the closing delimiter is line 2 — the error line follows
/// the post-gathering line_number, not the `<<` line.
#[test]
fn heredoc_dangling_operator_empty_body_line_two() {
    let (stdout, stderr, code) = rubash_file("echo x <<EOF>#c\nEOF\n");
    assert!(stdout.is_empty(), "stdout not empty: {stdout}");
    assert_eq!(
        stderr,
        "case.sh: line 2: syntax error near unexpected token `newline'\n\
         case.sh: line 2: `echo x <<EOF>#c'\n"
    );
    assert_eq!(code, Some(2));
}

/// A backslash-continued body line still counts PHYSICAL lines: closer is
/// line 4 (bo\ = 2, dy = 3, EOF = 4).
#[test]
fn heredoc_dangling_operator_counts_physical_body_lines() {
    let (stdout, stderr, code) = rubash_file("cat <<EOF>#c\nbo\\\ndy\nEOF\necho tail\n");
    assert!(stdout.is_empty(), "stdout not empty: {stdout}");
    assert_eq!(
        stderr,
        "case.sh: line 4: syntax error near unexpected token `newline'\n\
         case.sh: line 4: `cat <<EOF>#c'\n"
    );
    assert_eq!(code, Some(2));
}

/// A space before `#` puts the comment at a token boundary: the command
/// is complete and valid — heredoc feeds stdin, `x` prints.
#[test]
fn heredoc_word_spaced_comment_still_runs() {
    let (stdout, stderr, code) = rubash_file("echo x <<EOF #c\nbody\nEOF\necho rc=$?\n");
    assert_eq!(stdout, "x\nrc=0\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// `<<EOF>file` is a legal second redirection: heredoc plus stdout to the
/// file; `cat f` recovers `x`.
#[test]
fn heredoc_word_redirect_to_file_runs() {
    let (stdout, stderr, code) = rubash_file("echo x <<EOF>f\nbody\nEOF\ncat f\necho rc=$?\n");
    assert_eq!(stdout, "x\nrc=0\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// `<<EOF>&2#c`: the `>&2` dup is complete, the tail is a comment — valid.
#[test]
fn heredoc_word_complete_dup_with_comment_runs() {
    let (stdout, stderr, code) = rubash_file("echo x <<EOF>&2#c\nbody\nEOF\necho tail\n");
    assert_eq!(stdout, "tail\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// `#` INSIDE the delimiter word is word text (no token boundary): the
/// delimiter is `EOF#c', the gather runs to EOF and warns — unchanged.
#[test]
fn heredoc_hash_inside_delimiter_word_is_text() {
    let (stdout, stderr, code) = rubash_file("echo x <<EOF#c\nbody\nEOF\necho rc=$?\n");
    assert_eq!(stdout, "x\n");
    assert!(
        stderr
            .contains("warning: here-document at line 1 delimited by end-of-file (wanted `EOF#c')"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

/// Here-string family: `<<<EOF>#c` dangles the same way (LESS_LESS_LESS
/// WORD then GREATER then comment then NEWLINE).
#[test]
fn herestring_word_hash_tail_is_newline_error() {
    let (stdout, stderr, code) = rubash_file("echo x <<<EOF>#c\necho rc=$?\n");
    assert!(stdout.is_empty(), "stdout not empty: {stdout}");
    assert_eq!(
        stderr,
        "case.sh: line 1: syntax error near unexpected token `newline'\n\
         case.sh: line 1: `echo x <<<EOF>#c'\n"
    );
    assert_eq!(code, Some(2));
}

/// Without any heredoc, `>#c` already errored — but without the
/// offending-line echo. The family now byte-matches GNU's two-line form.
#[test]
fn plain_dangling_redirect_comment_two_line_diagnostic() {
    for script in [
        "echo x >#c\n",
        "echo x >>#c\n",
        "echo x 2>#c\n",
        "cat <#c\n",
    ] {
        let (stdout, stderr, code) = rubash_file(script);
        assert!(stdout.is_empty(), "{script:?} stdout: {stdout}");
        let first = script.lines().next().unwrap_or_default();
        assert_eq!(
            stderr,
            format!(
                "case.sh: line 1: syntax error near unexpected token `newline'\n\
                 case.sh: line 1: `{first}'\n"
            ),
            "{script:?} stderr"
        );
        assert_eq!(code, Some(2), "{script:?} code");
    }
}

/// Operator variants after the delimiter word: `>>`, `2<<`, `<<-`, and a
/// second dangling `<<` — all `newline' errors at the closer line.
#[test]
fn heredoc_dangling_operator_variants() {
    let cases: &[(&str, usize)] = &[
        ("echo x <<EOF>>#c\nbody\nEOF\necho tail\n", 3),
        ("echo x 2<<EOF>#c\nbody\nEOF\necho tail\n", 3),
        ("echo x <<-'K'>#c\n\tbody\n\tK\necho tail\n", 3),
        ("cat <<A <<#c\nbody\nA\necho tail\n", 3),
    ];
    for (script, line) in cases {
        let (stdout, stderr, code) = rubash_file(script);
        assert!(stdout.is_empty(), "{script:?} stdout: {stdout}");
        let first = script.lines().next().unwrap_or_default();
        assert_eq!(
            stderr,
            format!(
                "case.sh: line {line}: syntax error near unexpected token `newline'\n\
                 case.sh: line {line}: `{first}'\n"
            ),
            "{script:?} stderr"
        );
        assert_eq!(code, Some(2), "{script:?} code");
    }
}

/// Two heredocs on one command: the bodies gather in order and the line
/// counter ends at the LAST closer (line 5).
#[test]
fn heredoc_dangling_operator_multi_heredoc_last_closer_line() {
    let (stdout, stderr, code) = rubash_file("cat <<A <<B>#c\nbodyA\nA\nbodyB\nB\necho tail\n");
    assert!(stdout.is_empty(), "stdout not empty: {stdout}");
    assert_eq!(
        stderr,
        "case.sh: line 5: syntax error near unexpected token `newline'\n\
         case.sh: line 5: `cat <<A <<B>#c'\n"
    );
    assert_eq!(code, Some(2));
}

/// Trailing heredoc after a compound closer: `{ ...; }`, `while`/`done`,
/// `if`/`fi` and `(...)` — the dangling `>` is the missing-operand
/// diagnosis (`newline' at the closer line), not `unexpected token `>''.
#[test]
fn heredoc_dangling_operator_after_compound_closers() {
    for (script, line) in [
        ("{ :; } <<EOF>#c\nb\nEOF\necho tail\n", 3),
        ("while :; do :; done <<EOF>#c\nb\nEOF\necho tail\n", 3),
        ("if :; then :; fi <<EOF>#c\nb\nEOF\necho tail\n", 3),
        ("(:) <<EOF>#c\nb\nEOF\necho tail\n", 3),
    ] {
        let (stdout, stderr, code) = rubash_file(script);
        assert!(stdout.is_empty(), "{script:?} stdout: {stdout}");
        let first = script.lines().next().unwrap_or_default();
        assert_eq!(
            stderr,
            format!(
                "case.sh: line {line}: syntax error near unexpected token `newline'\n\
                 case.sh: line {line}: `{first}'\n"
            ),
            "{script:?} stderr"
        );
        assert_eq!(code, Some(2), "{script:?} code");
    }
}

/// GNU parses the newline-terminated list as one unit: `echo before` on
/// the SAME line as the failing command never runs.
#[test]
fn heredoc_dangling_operator_aborts_whole_list_line() {
    let (stdout, stderr, code) = rubash_file("echo before; echo x <<EOF>#c\nb\nEOF\necho tail\n");
    assert!(stdout.is_empty(), "stdout not empty: {stdout}");
    assert_eq!(
        stderr,
        "case.sh: line 3: syntax error near unexpected token `newline'\n\
         case.sh: line 3: `echo before; echo x <<EOF>#c'\n"
    );
    assert_eq!(code, Some(2));
}

/// An EOF-unterminated gather warns DURING gathering (make_cmd.c:626) —
/// before read_token returns the NEWLINE whose rejection prints the
/// syntax error — with the warning at the last physical body line and
/// the input-stream prefix (script name), not the parser's stream tag.
#[test]
fn heredoc_dangling_operator_eof_warning_precedes_error() {
    // Body read but never closed: last body line is 2.
    let (stdout, stderr, code) = rubash_file("echo x <<EOF>#c\nbody\n");
    assert!(stdout.is_empty(), "stdout not empty: {stdout}");
    assert_eq!(
        stderr,
        "case.sh: line 2: warning: here-document at line 1 delimited by end-of-file (wanted `EOF')\n\
         case.sh: line 2: syntax error near unexpected token `newline'\n\
         case.sh: line 2: `echo x <<EOF>#c'\n"
    );
    assert_eq!(code, Some(2));

    // No body at all: EOF on the header line itself.
    let (stdout, stderr, code) = rubash_file("echo x <<EOF>#c");
    assert!(stdout.is_empty(), "stdout not empty: {stdout}");
    assert_eq!(
        stderr,
        "case.sh: line 1: warning: here-document at line 1 delimited by end-of-file (wanted `EOF')\n\
         case.sh: line 1: syntax error near unexpected token `newline'\n\
         case.sh: line 1: `echo x <<EOF>#c'\n"
    );
    assert_eq!(code, Some(2));
}
