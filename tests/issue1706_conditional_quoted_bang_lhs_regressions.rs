//! rubash#1706 — a quoted `'!'` in the left-operand slot of a `[[ ]]`
//! binary primary was consumed as the negation operator, so the
//! primary always evaluated true.
//!
//! GNU ground truth: parse.y:3168-3181 CHECK_FOR_RESERVED_WORD maps a
//! word to BANG only when the word is unquoted (reserved words are
//! recognized from the raw token text); a quoted `'!'` is an ordinary
//! WORD operand of the primary (parse.y cond_term). Expectations
//! verified against GNU bash 5.3.20 (script files, dual runs).

use std::process::{Command, Stdio};

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1706_quoted_bang_lhs_string_primaries() {
    let (stdout, _, _) = run("[[ '!' == a ]]; echo \"eq-a=$?\"\n\
         [[ '!' != '!' ]]; echo \"ne-self=$?\"\n\
         [[ '!' == '!' ]]; echo \"eq-self=$?\"\n\
         [[ '!' = '!' ]]; echo \"single-eq-self=$?\"\n\
         [[ '!' == 'a*' ]]; echo \"eq-quoted-pat=$?\"\n\
         [[ '!' > a ]]; echo \"gt-a=$?\"\n\
         [[ '!' < a ]]; echo \"lt-a=$?\"\n");
    assert_eq!(
        stdout,
        "eq-a=1\nne-self=1\neq-self=0\nsingle-eq-self=0\neq-quoted-pat=1\ngt-a=1\nlt-a=0\n"
    );
}

#[test]
fn issue1706_quoted_bang_lhs_regex_and_numeric() {
    let (stdout, stderr, _) = run("[[ '!' =~ '!' ]]; echo \"re-quoted=$?\"\n\
         [[ '!' =~ . ]]; echo \"re-dot=$?\"\n\
         [[ '!' -eq 0 ]]; echo \"num-eq=$?\"\n");
    assert_eq!(stdout, "re-quoted=0\nre-dot=0\nnum-eq=1\n");
    // GNU: the numeric primary reaches evalexp, which rejects the
    // non-numeric operand with the arithmetic syntax diagnostic.
    assert!(
        stderr.contains("arithmetic syntax error: operand expected"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1706_quoted_bang_spellings_and_contexts() {
    let (stdout, _, _) = run("[[ \"!\" == a ]]; echo \"dq=$?\"\n\
         [[ \\! == a ]]; echo \"bs=$?\"\n\
         [[ ( '!' == a ) ]]; echo \"paren=$?\"\n\
         [[ x == x && '!' == a ]]; echo \"and=$?\"\n\
         [[ '!' == a || x == x ]]; echo \"or=$?\"\n");
    assert_eq!(stdout, "dq=1\nbs=1\nparen=1\nand=1\nor=0\n");
}

#[test]
fn issue1706_real_negation_unaffected() {
    let (stdout, _, _) = run("[[ '!' ]]; echo \"bare=$?\"\n\
         [[ ! '!' ]]; echo \"neg-bare=$?\"\n\
         [[ ! '!' == a ]]; echo \"neg-eq=$?\"\n\
         [[ ! x == a ]]; echo \"neg-word=$?\"\n\
         [[ ! -z x ]]; echo \"neg-unary=$?\"\n");
    assert_eq!(
        stdout,
        "bare=0\nneg-bare=1\nneg-eq=0\nneg-word=0\nneg-unary=0\n"
    );
}
