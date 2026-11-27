//! Issue unixwin/niubash#71 regression: two or more command substitutions
//! inside one double-quoted word must each expand on their own; the closing
//! paren of the first substitution must not leak as a literal.
//! GNU baseline: subst.c expands every $(...) span inside the quoted word and
//! concatenates the outputs (no re-splitting inside double quotes).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn two_comsubs_in_one_double_quoted_word() {
    let (stdout, stderr, code) = rubash(r#"echo "$(echo A)$(echo B)""#);
    assert_eq!(stdout, "AB\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn two_comsubs_with_separator_in_double_quoted_word() {
    let (stdout, _, _) = rubash(r#"echo "$(echo A)|$(echo B)""#);
    assert_eq!(stdout, "A|B\n");
}

#[test]
fn three_comsubs_in_double_quoted_word() {
    let (stdout, _, _) = rubash(r#"echo "$(echo A)$(echo B)$(echo C)""#);
    assert_eq!(stdout, "ABC\n");
}

#[test]
fn basename_dirname_join_in_double_quoted_word() {
    let script = r#"echo "$(basename /a/b/c.txt .txt)|$(dirname /a/b/c.txt)""#;
    let (stdout, _, _) = rubash(script);
    assert_eq!(stdout, "c|/a/b\n");
}

#[test]
fn quoted_assignment_with_two_comsubs() {
    let (stdout, _, _) = rubash(r#"v="$(echo A)$(echo B)"; echo "$v""#);
    assert_eq!(stdout, "AB\n");
}

#[test]
fn unquoted_assignment_with_two_comsubs() {
    let (stdout, _, _) = rubash(r#"u=$(echo A)$(echo B); echo "$u""#);
    assert_eq!(stdout, "AB\n");
}

#[test]
fn quoted_multi_comsub_word_stays_one_field() {
    let (stdout, _, _) = rubash(r#"for w in "$(echo A; echo B)$(echo C)"; do echo "[$w]"; done"#);
    assert_eq!(stdout, "[A\nBC]\n");
}

#[test]
fn single_comsub_in_double_quotes_unchanged() {
    let (stdout, _, _) = rubash(r#"echo "x $(echo y) z""#);
    assert_eq!(stdout, "x y z\n");
}

#[test]
fn nested_comsub_in_double_quotes_unchanged() {
    let (stdout, _, _) = rubash(r#"echo "$(echo $(echo hi))""#);
    assert_eq!(stdout, "hi\n");
}
