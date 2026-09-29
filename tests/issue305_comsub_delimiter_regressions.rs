//! rubash#305 lexmix-family regressions, in-comsub delimiter scan
//! (wt13/fixpack lane, Misc B).
//!
//! `$(cat <<EOF>#c)` — the raw-text comsub/heredoc scanners treated
//! `EOF>#c' as the heredoc DELIMITER: their delimiter-word break set only
//! stopped at whitespace and `;|&)'. GNU parse.y:5305 read_token_word
//! ends the delimiter word at ANY unquoted shell metacharacter
//! (parse.y:5688 shellbreak), so `<<EOF>#c)' declares delimiter `EOF',
//! `>' is a GREATER operator (parse.y:3667 shellmeta), and `#c)' is a
//! comment (parse.y:3630-3643) whose `)' never closes the substitution —
//! the input ends hunting `)' with "syntax error near unexpected token
//! `newline'".
//!
//! Fixed scanners (same invariant, whole class):
//! - parser/command_substitution.rs skip_command_substitution_heredoc:
//!   break set + comment-aware header-close scan.
//! - lexer/heredoc_scan.rs skip_heredoc_in_chars_with_closure: same.
//!
//! Verified against WSL GNU Bash 5.3.0 (2026-09-28, artifacts under
//! target/issue-suites/results/fixpackmiscb/). Residuals (recorded, not
//! fixed here): the gather warning for `<<EOF>#c)` inside a comsub still
//! names `EOF>#c' in one producer, and a redirection after the in-comsub
//! heredoc header (`$(cat <<EOF > /dev/null ...)` drops the redirect) —
//! both pre-existing on master.

use std::process::Command;

/// Run a script FILE in its own scratch directory; (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-miscb-{}-{}",
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

/// A well-formed in-comsub heredoc still works (guard the guard).
#[test]
fn comsub_heredoc_normal_body_still_works() {
    let (stdout, stderr, code) = rubash_file("echo $(cat <<'EOF'\nbody text\nEOF\n)\necho after\n");
    assert_eq!(stdout, "body text\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// `<<EOF>#c' with body lines: GNU reports the syntax error class while
/// hunting `)' — the delimiter must NOT have swallowed `>#c' (the old
/// scan gathered the whole file as heredoc body and reported a
/// here-document warning instead of the syntax error).
#[test]
fn comsub_heredoc_hash_tail_is_syntax_error_not_body() {
    let (stdout, stderr, code) =
        rubash_file("x=$(cat <<EOF>#c\nbody\nEOF\n)\necho \"x=[$x] after\"\n");
    assert_eq!(stdout, "", "nothing may run: the comsub never closes");
    assert_eq!(code, Some(2));
    assert!(
        stderr.contains("syntax error near unexpected token `newline'"),
        "stderr: {stderr}"
    );
}

/// The `)` inside the comment tail must not close the substitution: the
/// one-line form still fails closed (no output, status 2).
#[test]
fn comsub_heredoc_hash_tail_one_line_never_closes() {
    let (stdout, _stderr, code) = rubash_file("echo $(cat <<EOF>#c)\necho done\n");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
}

/// Metacharacter break in the delimiter scan: `<<EOF>` without a comment
/// still ends the delimiter at `>' (top-level rubash#305 family).
#[test]
fn comsub_heredoc_delimiter_ends_at_greater() {
    // `$(cat <<EOF>$(echo /dev/null) ...` keeps delimiter `EOF' and the
    // `>' operator; a well-formed variant: delimiter then newline body.
    let (stdout, stderr, code) = rubash_file("x=$(cat <<EOF\nbody\nEOF\n)\necho \"[$x]\"\n");
    assert_eq!(stdout, "[body]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
