//! Regression tests for rubash#3859:
//! ANSI-C `$'...'` containing an escaped quote `\'` followed by `)` inside
//! `$(...)` command substitutions (bare, double-quoted, nested, parameter
//! expansion, case, alias expansion, and multiline script mode).

use std::fs;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn rubash_c(script: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c")
}

fn rubash_script_file(script: &str) -> Output {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rubash_issue3859_{}_{}.sh",
        std::process::id(),
        nanos
    ));
    fs::write(&path, script).expect("write temp script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash script");
    let _ = fs::remove_file(&path);
    output
}

fn assert_ok_stdout(output: &Output, expected_stdout: &str, context: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{context}: expected exit 0, got {:?}, stdout={stdout:?}, stderr={stderr:?}",
        output.status.code()
    );
    assert_eq!(stdout, expected_stdout, "{context}: stdout mismatch");
}

#[test]
fn bare_comsub_ansic_escaped_quote_before_rparen() {
    let out = rubash_c(r"echo $(echo $'a\')b')");
    assert_ok_stdout(&out, "a')b\n", "a1 bare comsub");

    let out_file = rubash_script_file("echo $(echo $'a\\')b')\n");
    assert_ok_stdout(&out_file, "a')b\n", "a1 bare comsub script file");
}

#[test]
fn assignment_comsub_ansic_escaped_quote_before_rparen() {
    let out = rubash_c(r#"x=$(echo $'a\')b'); echo "$x""#);
    assert_ok_stdout(&out, "a')b\n", "a3 assignment bare comsub");

    let out_q = rubash_c(r#"x="$(echo $'a\')b')"; echo "$x""#);
    assert_ok_stdout(&out_q, "a')b\n", "assignment quoted comsub");
}

#[test]
fn double_quoted_comsub_ansic_escaped_quote_before_rparen() {
    let out = rubash_c(r#"echo "$(echo $'a\')b')""#);
    assert_ok_stdout(&out, "a')b\n", "a5 double-quoted comsub");

    let out_file = rubash_script_file("echo \"$(echo $'a\\')b')\"\n");
    assert_ok_stdout(&out_file, "a')b\n", "a5 double-quoted comsub script file");
}

#[test]
fn comsub_ansic_escaped_backslash_before_closing_quote() {
    let out_bare = rubash_c(r"echo $(echo $'a\\')");
    assert_ok_stdout(&out_bare, "a\\\n", "bare comsub escaped backslash");

    let out_quoted = rubash_c(r#"echo "$(echo $'a\\')""#);
    assert_ok_stdout(&out_quoted, "a\\\n", "quoted comsub escaped backslash");
}

#[test]
fn nested_comsub_ansic_escaped_quote_before_rparen() {
    let out_bare = rubash_c(r"echo $(echo $(echo $'a\')b'))");
    assert_ok_stdout(&out_bare, "a')b\n", "nested bare comsub");

    let out_quoted = rubash_c(r#"echo "$(echo "$(echo $'a\')b')")""#);
    assert_ok_stdout(&out_quoted, "a')b\n", "nested quoted comsub");
}

#[test]
fn param_expansion_comsub_ansic_escaped_quote_before_rparen() {
    let out_bare = rubash_c(r"unset x; echo ${x:-$(echo $'a\')b')}");
    assert_ok_stdout(&out_bare, "a')b\n", "param expansion bare comsub");

    let out_quoted = rubash_c(r#"unset x; echo "${x:-$(echo $'a\')b')}""#);
    assert_ok_stdout(&out_quoted, "a')b\n", "param expansion quoted comsub");
}

#[test]
fn case_inside_comsub_ansic_escaped_quote_before_rparen() {
    let out_bare = rubash_c(r"echo $(case x in x) echo $'a\')b' ;; esac)");
    assert_ok_stdout(&out_bare, "a')b\n", "case inside bare comsub");

    let out_quoted = rubash_c(r#"echo "$(case x in x) echo $'a\')b' ;; esac)""#);
    assert_ok_stdout(&out_quoted, "a')b\n", "case inside quoted comsub");
}

#[test]
fn alias_inside_comsub_ansic_escaped_quote_before_rparen() {
    let out_bare = rubash_c("shopt -s expand_aliases\nalias f=echo\necho $(f $'a\\')b')");
    assert_ok_stdout(&out_bare, "a')b\n", "alias inside bare comsub");

    let out_quoted = rubash_c("shopt -s expand_aliases\nalias f=echo\necho \"$(f $'a\\')b')\"");
    assert_ok_stdout(&out_quoted, "a')b\n", "alias inside quoted comsub");
}

#[test]
fn multiline_comsub_ansic_escaped_quote_before_rparen() {
    let out_bare = rubash_script_file("echo $(\necho $'a\\')\nb'\n)\n");
    assert_ok_stdout(&out_bare, "a') b\n", "multiline bare comsub");

    let out_quoted = rubash_script_file("echo \"$(\necho $'a\\')\nb'\n)\"\n");
    assert_ok_stdout(&out_quoted, "a')\nb\n", "multiline quoted comsub");
}

#[test]
fn multiple_ansic_quotes_inside_comsub() {
    let out = rubash_c(r"echo $(echo $'a\')' $'b\')')");
    assert_ok_stdout(&out, "a') b')\n", "multiple ansic quotes inside comsub");
}
