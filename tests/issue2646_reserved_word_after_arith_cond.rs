//! rubash#2646 regressions: a reserved word directly after a completed
//! `(( ))` arithmetic command or `[[ ]]` conditional command (no `;` or
//! newline between) must be recognized — GNU parse.y:5899-5940
//! `reserved_word_acceptable` includes ARITH_CMD and COND_END in the set
//! of tokens after which a reserved word is seen, so
//! `if ((expr)) then ((expr)) fi` (arith.tests:351, "allow reserved words
//! after an arithmetic command just because") and
//! `if [[ str ]] then [[ str ]] fi` (cond.tests:230) parse and run.
//!
//! rubash's post-compound connector guard in the command loop only
//! exempted `then`/`do` (the rubash#460 face), so a body-level `fi`,
//! `elif` or `else` directly after a `(( ))` / `[[ ]]` / `( )` compound
//! was reported as `syntax error near unexpected token 'fi'` and the
//! whole unit died (rc 2, nothing ran).
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle); the binary-path prefix of
//! stderr diagnostics is normalized away before comparison.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2646-{}-{}",
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

/// Run a script that must succeed silently: exact stdout, empty stderr,
/// rc 0.
fn check_ok(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "script: {script:?}, stderr: {stderr}");
    assert_eq!(stdout, expected_stdout, "script: {script:?}");
    assert_eq!(stderr, "", "script: {script:?}");
}

/// Run a script that must stay a syntax error: rc 2, nothing on stdout,
/// and the diagnostic names the expected offending token.
fn check_syntax_error(script: &str, token: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(2), "script: {script:?}, stderr: {stderr}");
    assert_eq!(stdout, "", "script: {script:?}");
    assert!(
        stderr.contains(&format!("syntax error near unexpected token `{token}'")),
        "script: {script:?}, stderr: {stderr}"
    );
}

#[test]
fn arith_condition_then_arith_body_upstream_shape() {
    // arith.tests:351 verbatim (expr unset -> both commands false, rc 0).
    check_ok("if ((expr)) then ((expr)) fi\necho after\n", "after\n");
}

#[test]
fn cond_condition_then_cond_body_upstream_shape() {
    // cond.tests:230 verbatim.
    check_ok("if [[ str ]] then [[ str ]] fi\necho after\n", "after\n");
}

#[test]
fn arith_body_runs_when_condition_true() {
    check_ok("x=0\nif ((1)) then ((x = 42)) fi\necho $x\n", "42\n");
}

#[test]
fn cond_body_after_arith_condition() {
    check_ok("if ((1)) then [[ y ]] fi\necho ok\n", "ok\n");
}

#[test]
fn arith_body_after_cond_condition() {
    check_ok("if [[ x ]] then ((2)) fi\necho ok\n", "ok\n");
}

#[test]
fn arith_body_after_plain_condition() {
    check_ok("if true; then ((2)) fi\necho ok\n", "ok\n");
}

#[test]
fn subshell_body_then_fi() {
    // `)' is in reserved_word_acceptable's set too (parse.y:5904): the
    // closer of any compound command leaves a command boundary.
    check_ok("if true; then (echo sub) fi\n", "sub\n");
}

#[test]
fn elif_after_arith_body() {
    check_ok(
        "if ((0)) then ((9)) elif ((1)) then ((2)) fi\necho ok\n",
        "ok\n",
    );
}

#[test]
fn else_after_arith_body() {
    check_ok(
        "x=0\nif ((0)) then ((x = 1)) else ((x = 2)) fi\necho $x\n",
        "2\n",
    );
}

#[test]
fn nested_if_body_after_cond() {
    check_ok(
        "if [[ x ]] then if ((1)) then ((2)) fi fi\necho ok\n",
        "ok\n",
    );
}

#[test]
fn then_after_word_stays_argument_error() {
    // After a plain WORD a reserved word is an argument, never a
    // boundary: `then' here is `true''s argument, so the condition list
    // runs into `fi' with no `then' seen — GNU names `fi'.
    check_syntax_error("if true then echo a; fi\n", "fi");
}

#[test]
fn juxtaposed_arith_commands_stay_error() {
    // Two commands with no connector are still a syntax error; the
    // reserved-word exemption must not swallow it. (The token NAMED
    // differs pre-existing — GNU names `2', rubash names `(' — only
    // the error-ness and rc are this issue's surface.)
    let (stdout, stderr, code) = rubash_file("((1)) ((2))\n");
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("syntax error near unexpected token"),
        "stderr: {stderr}"
    );
}

#[test]
fn stray_fi_after_subshell_stays_error() {
    // A `fi' that closes nothing is still the offending token.
    check_syntax_error("(echo x) fi\n", "fi");
}

#[test]
fn juxtaposed_brace_groups_stay_error() {
    check_syntax_error("{ echo a; } { echo b; }\n", "{");
}
