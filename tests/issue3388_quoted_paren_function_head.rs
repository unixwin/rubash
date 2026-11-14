//! rubash#3388 — a quoted `(` `)` pair after a command word is two WORDs,
//! never a function-definition head.
//!
//! GNU parse.y:3667 read_token: `(` / `)` are operator tokens only when
//! read unquoted (the shellmeta branch); a quoted or escaped paren is part
//! of a WORD (parse.y:5305 read_token_word). The function_def grammar
//! (parse.y:1054-1056 `WORD '(' ')'`) consumes the operator tokens only, so
//! `[ '(' ')'` is a plain simple command: the `[` builtin reports
//! `missing ']'` at run time (test.c), returns 2, and the script continues.
//!
//! The pre-fix tree matched the function head on token TEXT alone, so
//! `[ '(' ')'`, `echo '(' ')' x` and `test '(' ')'` parsed as
//! `NAME ( )` definitions and aborted the whole script with a syntax error.
//!
//! Expectations verified against GNU bash 5.3.20 with script files.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3388-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("t.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("t.sh")
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn unclosed_bracket_after_quoted_parens_is_a_runtime_error() {
    // The issue's regression seed.
    let script = "[ '(' a = a ')' ]\n\
                  [ $? -eq 0 ] && echo \"balanced-paren: PASS\" || echo \"balanced-paren: FAIL\"\n\
                  [ '(' ')'\n\
                  rc=$?\n\
                  [ \"$rc\" -eq 2 ] && echo \"missing-bracket-rc2: PASS\" || echo \"missing-bracket-rc2: FAIL (got $rc)\"\n\
                  echo \"survived-missing-bracket: PASS\"\n";
    let (stdout, stderr, code) = rubash_file("seed", script);
    assert_eq!(
        stdout,
        "balanced-paren: PASS\nmissing-bracket-rc2: PASS\nsurvived-missing-bracket: PASS\n"
    );
    assert_eq!(stderr, "t.sh: line 3: [: missing `]'\n");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_paren_pair_is_two_words_for_any_command() {
    let script = "echo '(' ')' x\n\
                  echo \"(\" \")\"\n\
                  [ \"(\" \")\"; echo \"dq=$?\"\n\
                  test '(' ')'; echo \"t=$?\"\n\
                  echo end\n";
    let (stdout, stderr, code) = rubash_file("words", script);
    assert_eq!(stdout, "( ) x\n( )\ndq=2\nt=2\nend\n");
    assert_eq!(
        stderr,
        "t.sh: line 3: [: missing `]'\n\
         t.sh: line 4: test: (: unary operator expected\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn unquoted_function_heads_still_define_functions() {
    let script = "f ( ) { echo \"f body\"; }\nf\n\
                  g() (echo \"g subshell\")\ng\n\
                  function h () { echo \"h body\"; }\nh\n";
    let (stdout, stderr, code) = rubash_file("heads", script);
    assert_eq!(stdout, "f body\ng subshell\nh body\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn dash_c_unclosed_bracket_continues() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "[ '(' ')'; echo rc=$?"])
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stdout, "rc=2\n");
    assert!(
        stderr.ends_with("line 1: [: missing `]'\n") && stderr.lines().count() == 1,
        "stderr: {stderr}"
    );
    assert_eq!(output.status.code(), Some(0));
}
