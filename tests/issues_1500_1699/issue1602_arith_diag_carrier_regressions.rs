//! Issue unixwin/rubash#1602 (gen-corpus R11): arithmetic error
//! diagnostics echoed the parser's internal subscript carrier encoding
//! — `a[<0x1e>30]` for `a[0]` — because the pre-evaluation passes
//! rewrite every subscript as `name[<SUBSCRIPT_CARRIER><hex of the
//! expanded subscript text>]` and the evalerror record rendered that
//! text verbatim.
//!
//! GNU owner: expr.c:1526-1535 evalerror echoes the frame's expression
//! as written; carrier bytes are an internal transport encoding and
//! never reach user-visible output. The record's render now decodes
//! each carrier span back to `[<subscript text>]` (the hex payload IS
//! the subscript text GNU prints: word expansion has already run, so
//! `a[$i]` arrives as `a[1]`, and bare names stay bare).
//!
//! Verified against GNU Bash 5.3.20 (script-file probes).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_rubash_file(script: &str, tag: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("issue1602_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&path).stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = cmd.output().expect("run rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Diagnostic lines with the program-name prefix stripped.
fn diag_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(|line| match line.find("line ") {
            Some(at) => line[at..].to_string(),
            None => line.to_string(),
        })
        .collect()
}

fn expect_one_diag(script: &str, tag: &str, expected: &str) -> Option<i32> {
    let (_stdout, stderr, code) = run_rubash_file(script, tag);
    assert!(
        !stderr.contains('\u{1e}'),
        "carrier byte leaked into diagnostic: {stderr:?}"
    );
    assert_eq!(diag_lines(&stderr), vec![expected.to_string()]);
    code
}

#[test]
fn issue1602_let_division_by_zero_names_source_subscript() {
    let code = expect_one_diag(
        "a=(10 20 30); z=0\nlet \"a[0] / z\"\n",
        "div",
        "line 2: let: a[0] / z: division by 0 (error token is \"z\")",
    );
    assert_eq!(code, Some(1));
}

#[test]
fn issue1602_let_modulo_by_zero_names_source_subscript() {
    let code = expect_one_diag(
        "a=(10 20 30); z=0\nlet \"a[0] % z\"\n",
        "mod",
        "line 2: let: a[0] % z: division by 0 (error token is \"z\")",
    );
    assert_eq!(code, Some(1));
}

#[test]
fn issue1602_variable_subscript_text_stays_verbatim() {
    // GNU echoes the expression as written: a bare subscript name is
    // not expanded in the expression text, a `$i` was already expanded
    // by the word expansion of the let operand itself.
    let (_o, stderr, code) = run_rubash_file(
        "a=(10 20 30); z=0; i=1\nlet \"a[i] / z\"\nlet \"a[i+1] / z\"\nlet \"a[$i] / z\"\n",
        "varsub",
    );
    assert!(
        !stderr.contains('\u{1e}'),
        "carrier byte leaked into diagnostic: {stderr:?}"
    );
    assert_eq!(
        diag_lines(&stderr),
        vec![
            "line 2: let: a[i] / z: division by 0 (error token is \"z\")".to_string(),
            "line 3: let: a[i+1] / z: division by 0 (error token is \"z\")".to_string(),
            "line 4: let: a[1] / z: division by 0 (error token is \"z\")".to_string(),
        ]
    );
    assert_eq!(code, Some(1));
}

#[test]
fn issue1602_assoc_key_names_source_text() {
    let code = expect_one_diag(
        "declare -A A=([k]=5); z=0\nlet \"A[k] / z\"\n",
        "assoc",
        "line 2: let: A[k] / z: division by 0 (error token is \"z\")",
    );
    assert_eq!(code, Some(1));
}

#[test]
fn issue1602_subscript_frame_error_stays_subscript_local() {
    // The nested-frame diagnostic (`1/z`, no `let:` prefix) was already
    // correct; the de-carrier render must not disturb it.
    let code = expect_one_diag(
        "a=(10 20 30); z=0\nlet \"a[1/z]\"\n",
        "nested",
        "line 2: 1/z: division by 0 (error token is \"z\")",
    );
    assert_eq!(code, Some(1));
}

#[test]
fn issue1602_expansion_form_control_unchanged() {
    // $(( )) echoes the frame with its trailing blank (GNU token `z `).
    let (_o, stderr, _c) = run_rubash_file("a=(10 20 30); z=0\necho $(( a[0] / z ))\n", "expctl");
    assert_eq!(
        diag_lines(&stderr),
        vec!["line 2: a[0] / z : division by 0 (error token is \"z \")".to_string()]
    );
}

#[test]
fn issue1602_no_carrier_byte_anywhere() {
    // Sweep: stdout of the failing command list carries on; stderr of
    // every shape must be carrier-free.
    let script = "a=(10 20 30); z=0\nlet \"a[0] / z\"\nlet \"a[1] % z\"\n(( a[2] / z ))\n";
    let (_stdout, stderr, _code) = run_rubash_file(script, "sweep");
    assert!(!stderr.contains('\u{1e}'), "carrier leaked: {stderr:?}");
    assert!(!stderr.contains("^^"), "caret form leaked: {stderr:?}");
    let _ = std::io::stdout().flush();
}
