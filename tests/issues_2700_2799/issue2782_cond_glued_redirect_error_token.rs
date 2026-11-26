//! rubash#2782 (split from #2750 at PR #2778's verification): the
//! conditional follow-on error walk truncates a glued redirect item.
//!
//! GNU's diagnostic for a missing-RHS conditional whose `]]` is glued
//! to a redirect names the whole run the reader had consumed:
//! `error_token_from_text` (parse.y:6772-6808) walks back from the
//! reader index to a break character (` \n\t;|&`), and the cond lexer
//! stops the offending word at the shellmeta `>` — so the reported
//! token is `==]]2` and the near token is `==]]2>` (io number and
//! operator included, target word excluded). For a COMPLETE
//! conditional followed by a glued redirect, the near walk covers the
//! whole glued item (`b]]2>f`, target included). rubash's follow-on
//! walk (src/parser/conditional_command.rs `cond_follower_text`)
//! contributes only the redirect operator's first byte.
//!
//! NOTE: the issue text says GNU names `]]2>f`; the 5.3.20 oracle
//! actually names `==]]2` / `==]]2>` for this shape. These pins encode
//! the oracle's bytes, verified by script-file probes.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2782-{}-{}",
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

fn check_error(script: &str, expected_stderr: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(
        code,
        Some(2),
        "script: {script}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(stdout, "", "script {script} wrote to stdout");
    assert_eq!(stderr, expected_stderr, "script: {script}");
}

/// The defect cell: missing RHS, `]]` glued to `2>f` — GNU names
/// `==]]2` (token) and `==]]2>` (near).
#[test]
fn glued_redirect_after_missing_rhs() {
    check_error(
        "[[ a ==]]2>f\n",
        "case.sh: line 1: unexpected token `==]]2', conditional binary operator expected\ncase.sh: line 1: syntax error near `==]]2>'\ncase.sh: line 1: `[[ a ==]]2>f'\n",
    );
}

/// Control: the same missing-RHS error with nothing glued on — GNU
/// names just `==]]`.
#[test]
fn plain_missing_rhs() {
    check_error(
        "[[ a ==]]\n",
        "case.sh: line 1: unexpected token `==]]', conditional binary operator expected\ncase.sh: line 1: syntax error near `==]]'\ncase.sh: line 1: `[[ a ==]]'\n",
    );
}

/// Control: a spaced redirect after the broken conditional does not
/// extend the walk — GNU still names `==]]`.
#[test]
fn spaced_redirect_after_missing_rhs() {
    check_error(
        "[[ a ==]] 2>f\n",
        "case.sh: line 1: unexpected token `==]]', conditional binary operator expected\ncase.sh: line 1: syntax error near `==]]'\ncase.sh: line 1: `[[ a ==]] 2>f'\n",
    );
}

/// The complete-conditional face of the same walk: `[[ a == b]]2>f`
/// parses the conditional, then trips on the glued redirect — GNU's
/// near token is the whole glued item `b]]2>f`.
#[test]
fn glued_redirect_after_complete_conditional() {
    check_error(
        "[[ a == b]]2>f\n",
        "case.sh: line 1: syntax error in conditional expression: unexpected token `>'\ncase.sh: line 1: syntax error near `b]]2>f'\ncase.sh: line 1: `[[ a == b]]2>f'\n",
    );
}
