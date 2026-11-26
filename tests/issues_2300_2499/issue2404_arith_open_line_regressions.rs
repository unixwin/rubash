//! rubash#2404 — script-level (non-eval) `$((` with an unclosed nested
//! group reported the EOF line; GNU reports the line where the
//! arithmetic group opened.
//!
//! GNU mechanics (bash 5.3.20, vendored source):
//! - The whole `$(( ... ))` is ONE matched pair: parse_comsub peeks the
//!   second `(` and hands the construct to the arithmetic
//!   parse_matched_pair scan (parse.y:4463-4470), inside which every
//!   further `(` is a grouping count of that same pair — never a new
//!   command context.
//! - parse_matched_pair records `start_lineno = line_number` on entry
//!   and reports it on EOF: `parser_error (start_lineno, "unexpected EOF
//!   while looking for matching `%c'")` (parse.y:3908-3915).
//! - The command-position `((` body is the same scan
//!   (parse.y:4970 -> parse_matched_pair P_ARITH), so a grouping paren
//!   opened on a later line still reports the `((` line.
//!
//! rubash's close-char scan (src/lexer/continuation.rs) pushed a bare
//! `(` inside a P_ARITH body as a fresh command-subshell delimiter
//! (report_open=false, command=true), so the innermost pending frame at
//! EOF was the grouping paren and the diagnostic took the yyerror EOF
//! line (and, for the `((` form, the "from `(' command" wording). The
//! eval-carried sibling shape was fixed earlier in skip.rs (ArithEof,
//! PR #2362); the script-level path runs through this generic scan.
//!
//! Expectations verified against GNU bash 5.3.20 (script files and -c,
//! 2026-11-01).

use std::io::Write;
use std::process::Command;

const EOF_PAREN: &str = "unexpected EOF while looking for matching `)'";

/// Run `script` as a script FILE and return (stdout, stderr lines with
/// the `<path>: ` prolog stripped to `line N: ...`, rc) — same harness
/// shape as tests/issue2342_eval_arith_eof_line_regressions.rs.
fn run_script(script: &str) -> (String, Vec<String>, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2404-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).expect("create probe");
    file.write_all(script.as_bytes()).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(|line| match line.find(": line ") {
            Some(idx) => line[idx + 2..].to_string(),
            None => line.to_string(),
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        output.status.code(),
    )
}

fn expect_script_error(script: &str, want_stdout: &str, want_line: usize) {
    let (stdout, stderr, rc) = run_script(script);
    assert_eq!(stdout, want_stdout, "stdout");
    assert_eq!(
        stderr,
        vec![format!("line {want_line}: {EOF_PAREN}")],
        "stderr"
    );
    assert_eq!(rc, Some(2), "script rc");
}

/// The issue seed: one-line script, grouping paren unclosed inside
/// `$((`. GNU reports the arithmetic open line (1), not the EOF line.
#[test]
fn issue2404_seed_reports_arith_open_line() {
    expect_script_error("echo $(( (echo\n", "", 1);
}

/// The report follows the `$((` line wherever it sits in the file;
/// complete commands before it still ran.
#[test]
fn issue2404_open_on_third_line_reports_third_line() {
    expect_script_error("echo before\ntrue\necho $(( (echo\n", "before\n", 3);
}

/// Multi-line arithmetic: the grouping paren opens on line 3, the
/// `$((` on line 2 — GNU reports line 2 (the pair's start_lineno),
/// not the grouping paren's line.
#[test]
fn issue2404_multiline_reports_digraph_line_not_group_line() {
    expect_script_error("echo x\necho $((\n  (echo\n", "x\n", 2);
}

/// Two nested grouping parens, both unclosed: still the one pair's
/// open line.
#[test]
fn issue2404_nested_groups_report_open_line() {
    expect_script_error("echo $(( (1 + (2\n", "", 1);
}

/// First grouping paren closed, second still open at EOF: the pair
/// never closed, so the open line stands.
#[test]
fn issue2404_second_group_open_reports_open_line() {
    expect_script_error("echo $(( (1) + (2\n", "", 1);
}

/// The command-position `((` body is the same arithmetic matched
/// pair: a grouping paren opened on line 2 reports the `((` line (1),
/// with the "matching `)'" wording (not "from `(' command").
#[test]
fn issue2404_dparen_group_on_later_line_reports_dparen_line() {
    expect_script_error("(( 1 +\n (2\n", "", 1);
}

/// The `-c` face shares the close-char scan verdict: GNU reports
/// line 1 for `bash -c 'echo $(( (echo'`.
#[test]
fn issue2404_dash_c_reports_open_line() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "echo $(( (echo"])
        .output()
        .expect("run rubash -c");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains(&format!("line 1: {EOF_PAREN}")),
        "stderr should name line 1: {stderr}"
    );
    assert_eq!(output.status.code(), Some(2), "-c rc");
}

/// Control: a balanced grouping inside `$((` still evaluates.
#[test]
fn issue2404_balanced_group_still_evaluates() {
    let (stdout, stderr, rc) = run_script("echo $(( (1 + 2) * 3 ))\n");
    assert_eq!(stdout, "9\n", "stdout");
    assert_eq!(stderr, Vec::<String>::new(), "stderr");
    assert_eq!(rc, Some(0), "script rc");
}

/// Control: a genuine unclosed command substitution still reports
/// the EOF line (parse.y:6891 yyerror route), unchanged.
#[test]
fn issue2404_comsub_eof_line_unchanged() {
    expect_script_error("echo $(echo\n", "", 2);
}

/// Control: the spaced `$( (` form is a comsub holding a subshell —
/// the EOF line, unchanged by the arithmetic-group rule.
#[test]
fn issue2404_spaced_comsub_subshell_keeps_eof_line() {
    expect_script_error("echo $( (echo\n", "", 2);
}
