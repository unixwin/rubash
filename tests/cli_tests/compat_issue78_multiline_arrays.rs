//! ISSUE #78 regression: a multi-line compound array assignment must stay a
//! single assignment word. The line-oriented collector has to keep reading
//! physical lines while a command-position `name=(` is unclosed (GNU parse.y),
//! instead of executing the array elements as commands.

use std::process::Command;

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .output()
        .expect("run multiline array probe");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn multiline_array_assignment_is_not_split_into_commands() {
    let (stdout, stderr, code) = run("plugins=(\n  git\n  completion\n)\necho ok");
    assert_eq!(stdout, "ok\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn multiline_array_assignment_keeps_elements() {
    let (stdout, stderr, code) =
        run("ab=(git\ncompletion)\nprintf '%s|%s\\n' \"${ab[0]}\" \"${ab[1]}\"");
    assert_eq!(stdout, "git|completion\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn multiline_array_assignment_via_declare() {
    let (stdout, stderr, code) =
        run("declare -a cd=(git\ncompletion)\nprintf '%s|%s\\n' \"${cd[0]}\" \"${cd[1]}\"");
    assert_eq!(stdout, "git|completion\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn multiline_append_compound_assignment() {
    let (stdout, stderr, code) = run("a=(7\n8\n)\nprintf '%s %s\\n' \"${a[0]}\" \"${a[1]}\"");
    assert_eq!(stdout, "7 8\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn comment_inside_multiline_array_body_is_not_a_closing_paren() {
    let (stdout, stderr, code) =
        run("a=(\n1 # c )\n2\n)\nprintf '%s|%s\\n' \"${a[0]}\" \"${a[1]}\"");
    assert_eq!(stdout, "1|2\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn separated_paren_is_still_a_syntax_error() {
    // `a= (1 2)` (space before the paren) must keep failing instead of the
    // continuation swallowing the next line.
    let (stdout, _stderr, code) = run("a= (1 2)\necho after");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
}

#[test]
fn unclosed_compound_assignment_at_eof_reports_gnu_eof_diagnostic() {
    // Run from a script file so the diagnostic carries the same
    // `<script>: line 1:` prefix GNU uses for unclosed compound assignments.
    let script_path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("issue78_unclosed.sh");
    std::fs::write(&script_path, "a=(\n1\n2\n").expect("write unclosed compound probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .output()
        .expect("run unclosed compound probe");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unexpected EOF while looking for matching `)'"));
    assert!(stderr.ends_with("line 1: unexpected EOF while looking for matching `)'\n"));
    assert_eq!(output.status.code(), Some(1));
}
