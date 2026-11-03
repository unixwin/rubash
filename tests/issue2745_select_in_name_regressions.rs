//! rubash#2745 (B6-R7): a `select` loop variable literally named `in`
//! was rejected — the parser treated the name slot's `in` as the list
//! keyword unconditionally and degraded the head to a simple command
//! (syntax error near `do'). GNU's grammar takes the WORD after
//! SELECT as the name (parse.y:979 `SELECT WORD ...'), so
//! `select in; do ...' and `select in in a b; do ...' both work.
use std::{
    env,
    io::Write,
    process::{Command, Stdio},
};

fn run_stdin(source: &str, stdin_text: &str) -> (String, String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(source)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -c");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(stdin_text.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn issue2745_select_variable_named_in_without_list() {
    // The head must parse (pre-fix: syntax error near `do', rc 2).
    // At EOF the menu loop ends without running the body (GNU same).
    let (stdout, stderr, code) = run_stdin("select in; do echo \"picked [$in]\"; break; done", "");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
}

#[test]
fn issue2745_select_variable_named_in_with_list() {
    let (stdout, stderr, code) = run_stdin(
        "select in in a b; do echo \"picked [$in]\"; break; done",
        "2\n",
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "picked [b]\n");
    assert!(stderr.contains("1) a"), "menu missing, stderr: {stderr}");
    assert!(stderr.contains("2) b"), "menu missing, stderr: {stderr}");
}

#[test]
fn issue2745_select_in_followed_by_words_still_errors_at_the_word() {
    // `select in a b' is not name+list: after the name `in' a word
    // that is neither `in' nor `do' is the syntax-error token.
    let (stdout, stderr, code) = run_stdin("select in a b; do echo x; break; done", "");
    assert_eq!(code, 2, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("syntax error near unexpected token `a'"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2745_ordinary_select_unchanged() {
    let (stdout, stderr, code) = run_stdin(
        "select v in a b; do echo \"picked [$v]\"; break; done",
        "1\n",
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "picked [a]\n");
}
