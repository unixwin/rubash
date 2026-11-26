//! Issue unixwin/rubash#1589 (arch-audit C, candidate D5): a script
//! whose tail holds BOTH an unclosed compound command and an unclosed
//! quote inside it reported TWO diagnostics — the compound's
//! `unexpected end of file from `if' command` (from running the
//! diagnostic route's line-prefix standalone) ahead of the quote's
//! `unexpected EOF while looking for matching `"'. GNU reports only
//! the quote diagnostic: its reader dies inside the compound's token
//! read (parse.y read_token_word), so the compound is the failed
//! command, none of it (nor anything else on its opening line) runs,
//! and the compound-EOF error is never reached.
//!
//! The fix retreats the unclosed-diagnostics route's prefix cut to
//! the outermost still-open compound's opening line
//! (`unclosed_compound_open_line` in script_driver.rs).
//!
//! Verified against GNU Bash 5.3.20 (script-file probes).

use std::process::{Command, Stdio};

fn run_rubash_file(script: &str, tag: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("issue1589_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn diag_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(|line| match line.find("line ") {
            Some(at) => line[at..].to_string(),
            None => line.to_string(),
        })
        .collect()
}

const QUOTE_LINE_3: &str = "line 3: unexpected EOF while looking for matching `\"'";

#[test]
fn issue1589_filed_shape_reports_only_the_quote() {
    let (stdout, stderr, code) = run_rubash_file(
        "echo first-ran\nif true; then\n  echo \"broken syntax here\nfi\necho never\n",
        "filed",
    );
    assert_eq!(stdout, "first-ran\n");
    assert_eq!(diag_lines(&stderr), vec![QUOTE_LINE_3.to_string()]);
    assert_eq!(code, Some(2));
}

#[test]
fn issue1589_while_compound_reports_only_the_quote() {
    let (stdout, stderr, code) =
        run_rubash_file("while true; do\n  echo \"broken\ndone\n", "while");
    assert_eq!(stdout, "");
    assert_eq!(
        diag_lines(&stderr),
        vec!["line 2: unexpected EOF while looking for matching `\"'".to_string()]
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1589_function_body_reports_only_the_quote() {
    let (stdout, stderr, code) = run_rubash_file("f() {\n  echo \"broken\n}\nf\n", "func");
    assert_eq!(stdout, "");
    assert_eq!(
        diag_lines(&stderr),
        vec!["line 2: unexpected EOF while looking for matching `\"'".to_string()]
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1589_nested_compounds_report_only_the_quote() {
    let (stdout, stderr, code) =
        run_rubash_file("if a; then\nwhile b; do\n echo \"broken\n", "nested");
    assert_eq!(stdout, "");
    assert_eq!(
        diag_lines(&stderr),
        vec!["line 3: unexpected EOF while looking for matching `\"'".to_string()]
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1589_same_line_command_does_not_run() {
    // GNU granularity is the failed command's opening LINE: a complete
    // command earlier on that same line does not run either.
    let (stdout, stderr, code) =
        run_rubash_file("echo a; if true; then\n echo \"broken\nfi\n", "sameline");
    assert_eq!(stdout, "");
    assert_eq!(
        diag_lines(&stderr),
        vec!["line 2: unexpected EOF while looking for matching `\"'".to_string()]
    );
    assert_eq!(code, Some(2));
}

#[test]
fn issue1589_closed_compound_before_still_runs() {
    // Control: a compound that CLOSED before the bad line is a complete
    // command — it runs, and only the quote diagnostic follows.
    let (stdout, stderr, code) = run_rubash_file(
        "echo before\nif true; then echo ok; fi\necho \"broken\n",
        "closed",
    );
    assert_eq!(stdout, "before\nok\n");
    assert_eq!(diag_lines(&stderr), vec![QUOTE_LINE_3.to_string()]);
    assert_eq!(code, Some(2));
}

#[test]
fn issue1589_unclosed_compound_without_quote_keeps_compound_eof() {
    // Control: with no inner matched-pair failure, the compound EOF
    // diagnostic is GNU's only diagnostic and must still fire.
    let (stdout, stderr, code) =
        run_rubash_file("echo first-ran\nif true; then\necho hi\n", "noquote");
    assert_eq!(stdout, "first-ran\n");
    assert_eq!(
        diag_lines(&stderr),
        vec![
            "line 4: syntax error: unexpected end of file from `if' command on line 2".to_string()
        ]
    );
    assert_eq!(code, Some(2));
}
