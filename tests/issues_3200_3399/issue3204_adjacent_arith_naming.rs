//! rubash#3204 regressions: when a second compound command starting
//! with `((` directly follows a completed `(( ))` or `[[ ]]`, GNU's
//! grammar rejects the line, but the token it names is the arithmetic
//! body's first token (`2`), not `(`. GNU reaches that position
//! because `parse_dparen` (parse.y:4895), gated by
//! `reserved_word_acceptable` (parse.y:4926 — ARITH_CMD and COND_END
//! are acceptable predecessors), reads the pair in via
//! `parse_arith_cmd` (parse.y:4965-4978), so when the syntax error
//! surfaces the lookahead is already the body's first token.
//!
//! rubash named `(`: its scanner `((` fold gate consults
//! `reserved_word_position()`, which lacked the ARITH_CMD/COND_END
//! arms (#3203), so after a compound the `((` never folded and the
//! parser errored on the bare first `(`. These pins sit on top of the
//! #3203 scanner fix; the negative control pins the fold gate's other
//! side — an argument-position `((` must still NOT fold.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle); on the unfixed tree the
//! four defect cells name `(` and fail, every control passes.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3204-{}-{}",
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

/// Run a script that must succeed: exact stdout, empty stderr, rc 0.
fn check_ok(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "script: {script:?}, stderr: {stderr}");
    assert_eq!(stdout, expected_stdout, "script: {script:?}");
    assert_eq!(stderr, "", "script: {script:?}");
}

/// Run a script that must stay a syntax error: rc 2, nothing on stdout,
/// the diagnostic names the expected offending token, and the source
/// line is echoed back (both diagnostic lines match GNU's shape).
fn check_syntax_error(script: &str, token: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(2), "script: {script:?}, stderr: {stderr}");
    assert_eq!(stdout, "", "script: {script:?}");
    assert!(
        stderr.contains(&format!("syntax error near unexpected token `{token}'")),
        "script: {script:?}, stderr: {stderr}"
    );
    let first_line = script.lines().next().unwrap_or("");
    assert!(
        stderr.contains(first_line),
        "script: {script:?}, stderr should echo the source line, stderr: {stderr}"
    );
}

#[test]
fn adjacent_arith_after_arith_names_body_token() {
    // Probe cell j1.
    check_syntax_error("((1)) ((2))\necho after\n", "2");
}

#[test]
fn adjacent_arith_after_cond_names_body_token() {
    // Probe cell j2.
    check_syntax_error("[[ x ]] ((2))\necho after\n", "2");
}

#[test]
fn adjacent_arith_inside_subshell_names_body_token() {
    // Probe cell j8.
    check_syntax_error("( ((1)) ((2)) )\necho after\n", "2");
}

#[test]
fn triple_adjacent_arith_names_second_body_token() {
    // Probe cell j9: the error surfaces at the SECOND compound already.
    check_syntax_error("((1)) ((2)) ((3))\necho after\n", "2");
}

#[test]
fn argument_position_double_paren_does_not_fold() {
    // Negative control for the fold gate: after an ordinary argument
    // word `]]`, `((2))` is not in reserved-word position, so no fold
    // may happen and the parser names the bare `(` — exactly GNU.
    // (The error is on line 2 of the script; the echo line proves it.)
    let script = "f() { printf \"<%s>\\n\" \"$@\"; }\nf ]] ((2))\n";
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(2), "script: {script:?}, stderr: {stderr}");
    assert_eq!(stdout, "", "script: {script:?}");
    assert!(
        stderr.contains("syntax error near unexpected token `('"),
        "script: {script:?}, stderr: {stderr}"
    );
    assert!(
        stderr.contains("f ]] ((2))"),
        "script: {script:?}, stderr should echo the source line, stderr: {stderr}"
    );
}

#[test]
fn lone_arith_command_runs() {
    // The fold itself is healthy: a lone arithmetic command evaluates.
    check_ok("((2))\n", "");
}

#[test]
fn arith_expansion_still_expands() {
    check_ok("echo $((2))\n", "2\n");
}
