//! Issue unixwin/rubash#1603 (gen-corpus R12): in the `(( ))` command,
//! a quoted INDEXED-array subscript was framed on the outer expression
//! — `((: a['k'] : arithmetic syntax error: operand expected (error
//! token is \"'k'] \")` — instead of the subscript's own evalexp frame.
//!
//! Root cause: `has_bare_single_quote` treated only ASSOCIATIVE
//! subscripts' quotes as part of a subscript; an indexed subscript's
//! quotes short-circuited the whole evaluation before the parser ran,
//! and the caller's text heuristics framed the diagnostic on the outer
//! text. GNU evaluates every subscript through array_expand_index's own
//! evalexp frame (arrayfunc.c:1356-1391), whose evalerror names the
//! subscript text (expr.c:1526-1535).
//!
//! Verified against GNU Bash 5.3.20 (script-file probes).

use std::process::{Command, Stdio};

fn run_rubash_file(script: &str, tag: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("issue1603_{}_{}.sh", tag, std::process::id()));
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

#[test]
fn issue1603_quoted_indexed_subscript_reports_subscript_frame() {
    let (_o, stderr, _c) = run_rubash_file("a=(10 20 30)\n(( a['k'] ))\n", "plain");
    assert_eq!(
        diag_lines(&stderr),
        vec![
            "line 2: 'k': arithmetic syntax error: operand expected (error token is \"'k'\")"
                .to_string()
        ]
    );
}

#[test]
fn issue1603_quoted_subscript_expression_reports_whole_subscript() {
    let (_o, stderr, _c) = run_rubash_file("a=(10 20 30)\n(( a['k' + 1] ))\n", "expr");
    assert_eq!(
        diag_lines(&stderr),
        vec![
            "line 2: 'k' + 1: arithmetic syntax error: operand expected (error token is \"'k' + 1\")"
                .to_string()
        ]
    );
}

#[test]
fn issue1603_quoted_subscript_assignment_reports_subscript_frame() {
    let (_o, stderr, _c) = run_rubash_file("a=(10 20 30)\n(( a['k'] = 5 ))\n", "assign");
    assert_eq!(
        diag_lines(&stderr),
        vec![
            "line 2: 'k': arithmetic syntax error: operand expected (error token is \"'k'\")"
                .to_string()
        ]
    );
}

#[test]
fn issue1603_bare_quoted_operand_keeps_outer_frame() {
    // Control: a quote that is NOT inside a subscript is still the
    // outer frame's operand error, byte-identical to GNU (including
    // the trailing blank inside the token).
    let (_o, stderr, _c) = run_rubash_file("(( '1' ))\n", "bare");
    assert_eq!(
        diag_lines(&stderr),
        vec![
            "line 1: ((: '1' : arithmetic syntax error: operand expected (error token is \"'1' \")"
                .to_string()
        ]
    );
}

#[test]
fn issue1603_quote_after_closed_subscript_keeps_outer_frame() {
    // Control: only the bracket span is the subscript's; a quote after
    // it is a bare operand quote again.
    let (_o, stderr, _c) = run_rubash_file("a=(10 20 30)\n(( a[0] + '1' ))\n", "after");
    assert_eq!(
        diag_lines(&stderr),
        vec![
            "line 2: ((: a[0] + '1' : arithmetic syntax error: operand expected (error token is \"'1' \")"
                .to_string()
        ]
    );
}

#[test]
fn issue1603_double_quoted_subscript_still_evaluates() {
    // Control: `a["k"]` — expand_arith_string strips the double quotes
    // inside the subscript; k resolves as a variable (0), no diagnostic.
    let (stdout, stderr, _c) = run_rubash_file("a=(10 20 30)\n(( a[\"k\"] ))\necho rc=$?\n", "dq");
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1603_assoc_quoted_key_still_binds() {
    // Control: the associative exception the predicate always had —
    // a quoted key with a space is data, not an operand error.
    let (stdout, stderr, _c) = run_rubash_file(
        "declare -A A=()\n(( A['a b'] = 7 ))\necho \"v=${A['a b']}\"\n",
        "assoc",
    );
    assert_eq!(stdout, "v=7\n");
    assert_eq!(stderr, "");
}
