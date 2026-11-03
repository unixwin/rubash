//! rubash#2748 (B6-R10): `function f > /dev/null' ran `function' as
//! an external command (127) — a redirection in function-body
//! position fell out of the definition parser into the word path.
//! GNU's function_def (parse.y) admits redirections only AFTER the
//! body (`function_body: shell_command [redirection_list]`), so a
//! redirection where the body belongs is a syntax error naming the
//! operator (an fd-led form names the fd digit). The commitment is
//! the same for the `name ()' form without the `function' keyword.
use std::{env, process::Command};

fn run(source: &str) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(source)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn assert_syntax_error_near(source: &str, token: &str) {
    let (stdout, stderr, code) = run(source);
    assert_eq!(code, 2, "source: {source}\nstdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "", "source: {source}");
    assert!(
        stderr.contains(&format!("syntax error near unexpected token `{token}'")),
        "source: {source}\nexpected blame `{token}', stderr: {stderr}"
    );
}

#[test]
fn issue2748_function_keyword_redirect_in_body_position() {
    assert_syntax_error_near("function f > /dev/null", ">");
    assert_syntax_error_near("function f > /dev/null { echo hi; }", ">");
    assert_syntax_error_near("function f < /dev/null { echo hi; }", "<");
}

#[test]
fn issue2748_fd_led_redirect_names_the_digit() {
    assert_syntax_error_near("function f 2>/dev/null { echo hi; }", "2");
}

#[test]
fn issue2748_paren_forms_redirect_in_body_position() {
    assert_syntax_error_near("function f() > /dev/null { echo hi; }", ">");
    assert_syntax_error_near("f() > /dev/null { echo hi; }", ">");
}

#[test]
fn issue2748_valid_definitions_unchanged() {
    let (stdout, stderr, code) = run("function f { echo hi; }\nf");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hi\n");

    let (stdout, stderr, code) = run("f() { echo hi; }\nf");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hi\n");

    // Redirections AFTER the body are the legal function_def form.
    let (stdout, stderr, code) = run("function f { echo hi; } > /dev/null\nf\necho after");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "after\n");

    // A compound command is a legal body without braces.
    let (stdout, stderr, code) = run("function f if true; then echo yes; fi\nf");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "yes\n");
}
