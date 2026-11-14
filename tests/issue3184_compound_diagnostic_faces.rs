//! Regression pins for rubash#3184 — three diagnostic faces of
//! compound constructs.
//!
//! B4: a syntax error inside a compound assignment `a=( ... )`
//! names the offending token the way GNU's read_token word
//! classification does (parse.y read_token's operator table): the
//! longest operator spelling at the error point (`<>`, `>>`, `<<<`,
//! `&&`, `;&`, ...), and for a redirection operator directly after
//! an all-digit word, the io_number — the DIGITS are the token
//! (`a=( 2>)` reports `2'). rubash used to name a single byte.
//!
//! B6: a command substitution whose body hits EOF mid-construct
//! echoes the enclosing source line GNU's print_offending_line
//! (parse.y:6814) prints — including when the stored body text is
//! empty and the line must come from the reader unit that carried
//! the substitution.
//!
//! B7: an arithmetic frame that re-evaluates a variable's TEXT
//! (expr.c:1241 expr_streval -> subexpr) parses the text VERBATIM:
//! only leading blanks are skipped when the error position is
//! computed (expr.c:1528-1530), so `A="4 + "` reports the frame
//! `4 + ` with error token `+ ` — rubash used to trim the value and
//! report the frame `4 +` with token `+`.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts under LC_ALL=C.UTF-8, measured against the oracle
//! binary (lane dossier probe battery, shapes 3184-B4a/B4b/B6a/B7a/
//! B7b, plus the operator matrix probe).
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3184-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::canonicalize(&dir).expect("canonical root")
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn expect_near(root: &Path, body: &str, token: &str) {
    let out = run(root, body);
    // The compound-assignment error surfaces as the command's own
    // failure (status 1), not a whole-file parse abort (status 2) —
    // measured against the oracle for every shape below.
    assert_eq!(out.rc, 1, "body: {body:?}");
    assert_eq!(
        out.stderr,
        format!(
            "case.sh: line 1: syntax error near unexpected token `{token}'\n\
             case.sh: line 1: `{body}'\n"
        ),
        "body: {body:?}"
    );
}

/// B4a (battery form): the first offending operator in the element
/// list is named with its full spelling.
#[test]
fn compound_assignment_names_full_operator() {
    let root = scratch("b4a");
    expect_near(&root, "metas=( <> < > ! )", "<>");
}

/// B4b (battery form): subscripted elements take the same naming.
#[test]
fn compound_assignment_subscripted_names_full_operator() {
    let root = scratch("b4b");
    expect_near(&root, "metas=( [1]=<> [2]=< )", "<>");
}

/// The read_token operator table: longest match at the error
/// point, and the io_number rule for digit-led redirections.
#[test]
fn compound_assignment_operator_matrix() {
    let root = scratch("b4m");
    for (body, token) in [
        ("a=( >> )", ">>"),
        ("a=( <<< )", "<<<"),
        ("a=( << )", "<<"),
        ("a=( <& )", "<&"),
        ("a=( >| )", ">|"),
        ("a=( &> )", "&>"),
        ("a=( && )", "&&"),
        ("a=( ;& )", ";&"),
        ("a=( 2> )", "2"),
        ("a=( 3<> )", "3"),
    ] {
        expect_near(&root, body, token);
    }
}

/// B6a (battery form, file mode): the comsub body's EOF error
/// echoes the enclosing line verbatim.
#[test]
fn comsub_body_eof_echoes_enclosing_line() {
    let root = scratch("b6a");
    let out = run(&root, ": $( for z in 1 2 3; do )\n");
    assert_eq!(out.rc, 2);
    assert_eq!(
        out.stderr,
        "case.sh: line 1: syntax error near unexpected token `)'\n\
         case.sh: line 1: `: $( for z in 1 2 3; do )'\n"
    );
}

/// B7b (battery form): the nested frame is the variable's text
/// verbatim — trailing blank included — and the error token runs
/// to the end of the frame.
#[test]
fn arith_nested_frame_is_verbatim() {
    let root = scratch("b7b");
    let out = run(&root, "A=\"4 + \"\necho $(( A ))\n");
    assert_eq!(out.rc, 1);
    assert_eq!(
        out.stderr,
        "case.sh: line 2: 4 + : arithmetic syntax error: operand expected (error token is \"+ \")\n"
    );
}

/// B7a (battery form): the same verbatim frame one nesting level
/// up — the outer expression's own text is not the frame.
#[test]
fn arith_nested_frame_in_larger_expression() {
    let root = scratch("b7a");
    let out = run(&root, "A=\"4 + \"\necho $(( ( 4 + A ) + 4 ))\n");
    assert_eq!(out.rc, 1);
    assert_eq!(
        out.stderr,
        "case.sh: line 2: 4 + : arithmetic syntax error: operand expected (error token is \"+ \")\n"
    );
}

/// Guard: a blank-padded value still EVALUATES trimmed-clean —
/// only the diagnostic frame is verbatim.
#[test]
fn arith_padded_value_evaluates() {
    let root = scratch("b7g");
    let out = run(&root, "A=\" 3 \"\necho $(( A + 1 ))\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stdout, "4\n");
    assert_eq!(out.stderr, "");
}
