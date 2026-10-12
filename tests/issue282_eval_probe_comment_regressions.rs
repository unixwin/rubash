//! Issue rubash#282 regression: the eval EOF-shape probe
//! (`lexer::unclosed_comsub_eof_shape`, added by rubash#318/#333) was
//! comment-blind — a `#' at a token boundary must comment out the rest of
//! its physical line (GNU parse.y:3630-3643 read_token -> parse_comment,
//! word-initial test :3937-3940), and quote or paren characters inside that
//! comment text are data, not scanners input.
//!
//! Found by the modernish fatal.sh battery (rubash#282): its FTL_CSCMTQUOT
//! probe is `eval ': || $( : # "\n)'` — the in-comment `"' flipped the
//! probe's double-quote state, swallowed the newline and the closing `)',
//! and init died on a fabricated "unexpected EOF while looking for matching
//! `)'" with MSH_FTL_DEBUG tracing nothing (the battery aborted before any
//! traceable statement). Bisect: first bad commit 206dc040 (rubash#318).
//!
//! GNU mechanics (all pinned byte-for-byte vs WSL GNU Bash 5.3.0; probes in
//! the temp/282 lane, scripts c1-c7):
//! - a comment inside a command-substitution body never opens a quote and
//!   never hides the `)' that closes the substitution on the NEXT line
//!   (modernish fatal.sh:514-518);
//! - a same-line comment DOES eat a same-line `)' (p2: `$(echo a # b)' is
//!   an unclosed comsub — plain EOF wording, eval returns 2, script
//!   continues);
//! - `$#' after a parameter is word text, never a comment introducer;
//! - a backslash-escaped space is word text, so `a\ #' keeps `#' mid-word;
//! - a `(' inside a comment is not a paren; a real unclosed comsub on a
//!   later line still reports the plain EOF wording, not a token error;
//! - `echo ( # c' names `newline' — the `#' right after the shifted
//!   function-definition paren is word-initial, comments out the rest of
//!   the line, and the next token the grammar sees is the newline.

use std::io::Write;
use std::process::Command;

/// Run `script` as a script FILE and return (stdout,
/// stderr-tail-lines-normalized, rc). The normalization strips the
/// invocation-dependent `<path>: ` prolog so assertions only see
/// GNU-identical text.
fn run_script(script: &str) -> (String, Vec<String>, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i282-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).expect("create probe");
    file.write_all(script.as_bytes()).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(|line| match line.find(": eval: ") {
            Some(idx) => line[idx + 2..].to_string(),
            None => line.to_string(),
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        output.status.code(),
    )
}

/// The modernish FTL_CSCMTQUOT probe verbatim (fatal.sh:514-518, reduced):
/// the `"' sits inside a comment, and the `)' closing the substitution is
/// on the next line. GNU parses the eval string cleanly.
#[test]
fn comment_quote_does_not_swallow_next_line_close_paren() {
    let (stdout, stderr, rc) = run_script("f() {\n  eval ': || $( : # \"\n)'\n}\nf\necho after\n");
    assert_eq!(stdout, "after\n", "stdout");
    assert_eq!(stderr, Vec::<String>::new(), "stderr");
    assert_eq!(rc, Some(0));
}

/// Same construct with an apostrophe in the comment (single-quote state must
/// not flip either); double-quoted eval string variant.
#[test]
fn comment_apostrophe_does_not_open_single_quote() {
    let (stdout, stderr, rc) =
        run_script("f() {\n  eval \": || \\$( : # don't\n)\"\n}\nf\necho after\n");
    assert_eq!(stdout, "after\n", "stdout");
    assert_eq!(stderr, Vec::<String>::new(), "stderr");
    assert_eq!(rc, Some(0));
}

/// GNU probe p2: a same-line comment eats the same-line `)' — the comsub
/// genuinely never closes. Plain EOF wording, eval returns 2, the script
/// continues (trailing `echo after' owns the exit status).
#[test]
fn same_line_comment_eats_close_paren_reports_plain_eof() {
    let (stdout, stderr, rc) = run_script("f() {\n  eval 'echo $(echo a # b)'\n}\nf\necho after\n");
    assert_eq!(stdout, "after\n", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 3: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(0));
}

/// `$#' after a parameter expansion is word text (GNU probe p1: `echo $#'
/// inside the comsub still runs and prints the positional count).
#[test]
fn hash_after_dollar_is_word_text_not_comment() {
    let (stdout, stderr, rc) = run_script("eval 'echo $(echo $#)'\n");
    assert_eq!(stdout, "0\n", "stdout");
    assert_eq!(stderr, Vec::<String>::new(), "stderr");
    assert_eq!(rc, Some(0));
}

/// GNU probe p9: a backslash-escaped space is word text, so `a\ #' keeps
/// `#' mid-word — the rest of the line is printed, not commented.
#[test]
fn hash_after_escaped_space_is_word_text() {
    let (stdout, stderr, rc) = run_script("eval 'echo a\\ #b'\n");
    assert_eq!(stdout, "a #b\n", "stdout");
    assert_eq!(stderr, Vec::<String>::new(), "stderr");
    assert_eq!(rc, Some(0));
}

/// A `(' inside a comment is not a paren: no token error from it; the REAL
/// unclosed comsub on the next line keeps the plain EOF wording. The
/// complete first string line still ran (`a` printed) before eval returned
/// 2 (evalstring.c:359-451); the script continues.
#[test]
fn comment_paren_hidden_real_comsub_still_reports_plain_eof() {
    let (stdout, stderr, rc) =
        run_script("f() {\n  eval 'echo a # (\necho $(b'\n}\nf\necho after\n");
    assert_eq!(stdout, "a\nafter\n", "stdout");
    assert_eq!(
        stderr,
        vec!["eval: line 4: unexpected EOF while looking for matching `)'".to_string()],
        "stderr"
    );
    assert_eq!(rc, Some(0));
}

/// GNU probe p7: `echo ( # c' — the shifted function-definition paren is
/// followed by a word-initial `#' comment, so the yacc error names
/// `newline' and echoes the offending line; the failing yyparse is the main
/// one, so the error is contained to the eval (script continues).
#[test]
fn comment_after_invalid_paren_names_newline_token() {
    let (stdout, stderr, rc) = run_script("f() {\n  eval 'echo ( # c'\n}\nf\necho after\n");
    assert_eq!(stdout, "after\n", "stdout");
    assert_eq!(
        stderr,
        vec![
            "eval: line 2: syntax error near unexpected token `newline'".to_string(),
            "eval: line 2: `echo ( # c'".to_string(),
        ],
        "stderr"
    );
    assert_eq!(rc, Some(0));
}
