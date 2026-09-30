//! Issue rubash#360 — four over-lax parser admissions from the ShellCheck
//! unit-test corpus (`bash -n` rc parity vs WSL GNU Bash 5.3.0): rubash
//! accepted constructs GNU rejects.
//!
//! GNU anchors (bash.git @b4608166):
//! 1. `[[ 3 \< 4 ]]` — parse.y:5156-5174 cond_term's binary-operator slot
//!    matches the parser's word TEXT (quote removal has not run), so a
//!    quoted `'=='`/`'<'` or escaped `\<`/`\==` matches no operator and
//!    takes the error branch at parse.y:5179-5191:
//!    `unexpected token `\<', conditional binary operator expected`
//!    + `syntax error near `\<'` + source line, rc 2. Verified matrix:
//!    `[[ 3 '<' 4 ]]`, `[[ 3 '==' 3 ]]`, `[[ 3 \== 3 ]]`, `[[ 3 \> 2 ]]`
//!    all rejected; `[[ 3 < 4 ]]`, `[[ 3 == 3 ]]` accepted.
//! 2. `true | ! true` — parse.y:1408-1413 `pipeline_command: ... | BANG
//!    pipeline_command`: BANG may only prefix a whole pipeline_command,
//!    never follow `|` inside a pipe_sequence. `true && ! true`,
//!    `true; ! true`, `if ! true`, `true | { ! true; }` stay legal.
//! 3./4. `var=( (1 2) (3 4) )`, `var=(1 [2]=(3 4))` —
//!    parse.y:7104-7160 parse_compound_assignment's element loop accepts
//!    only WORD and ASSIGNMENT_WORD tokens; a `(` token (element position,
//!    mid-word, or after `[i]=`) hits the yyerror:
//!    `syntax error near unexpected token `('` + source line, rc 1.
//!    Quoted `'('`/`"("` and escaped `\(` stay legal word content;
//!    `$((...))`, backticks and `<(cmd)`/`>(cmd)` process substitutions
//!    are consumed as WORD parts and stay legal.
//!
//! Verification: the six corpus snippets (Analytics_0255, Parser_2069,
//! Parser_2144/2145/2146, ShellSupport_1846) now match GNU rc; full
//! 2164-snippet corpus rerun: 2163/2164 parity, the single remainder
//! (Parser_1985) is #361's over-strict `!(` case. Legal-form matrix and
//! the 83-suite slices (cond/array/errors/dstack) byte-identical to a
//! same-harness rerun on master.

use std::process::Command;

fn rubash_n(script: &str) -> (String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-n")
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -n");
    (
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// `[[ 3 \< 4 ]]` — escaped operator rejected with GNU's exact messages.
#[test]
fn escaped_conditional_operator_rejected() {
    let (stderr, code) = rubash_n("[[ 3 \\< 4 ]]");
    assert_eq!(
        stderr,
        "bash: -c: line 1: unexpected token `\\<', conditional binary operator expected\n\
         bash: -c: line 1: syntax error near `\\<'\n\
         bash: -c: line 1: `[[ 3 \\< 4 ]]'\n"
    );
    assert_eq!(code, Some(2));
}

/// Quoted operator words are rejected the same way (GNU prints the raw
/// quoted form in the messages).
#[test]
fn quoted_conditional_operator_rejected() {
    let (stderr, code) = rubash_n("[[ 3 '<' 4 ]]");
    assert!(stderr.contains("unexpected token `'<'', conditional binary operator expected"));
    assert_eq!(code, Some(2));
    let (stderr, code) = rubash_n("[[ 3 '==' 3 ]]");
    assert!(stderr.contains("unexpected token `'=='', conditional binary operator expected"));
    assert_eq!(code, Some(2));
}

/// Unescaped/unquoted operators keep working.
#[test]
fn plain_conditional_operators_accepted() {
    let (stderr, code) = rubash_n("[[ 3 < 4 ]] && [[ 3 == 3 ]] && [[ 3 -lt 4 ]]");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// `true | ! true` — BANG after a pipe is a syntax error.
#[test]
fn bang_after_pipe_rejected() {
    let (stderr, code) = rubash_n("true | ! true");
    assert_eq!(
        stderr,
        "bash: -c: line 1: syntax error near unexpected token `!'\n"
    );
    assert_eq!(code, Some(2));
}

/// Every other BANG position stays legal.
#[test]
fn bang_positions_legal() {
    for script in [
        "! true",
        "true && ! true",
        "true; ! true",
        "if ! true; then :; fi",
        "true | { ! true; }",
        "! ! true",
    ] {
        let (stderr, code) = rubash_n(script);
        assert_eq!(stderr, "", "{script}");
        assert_eq!(code, Some(0), "{script}");
    }
}

/// Nested paren array element rejected.
#[test]
fn paren_array_element_rejected() {
    let (stderr, code) = rubash_n("var=( (1 2) (3 4) )");
    assert_eq!(
        stderr,
        "bash: -c: line 1: syntax error near unexpected token `('\nbash: -c: line 1: `var=( (1 2) (3 4) )'\n"
    );
    assert_eq!(code, Some(1));
}

/// `[i]=(...)` compound element rejected.
#[test]
fn subscript_compound_element_rejected() {
    for script in ["var=( 1 [2]=(3 4) )", "var=(1 [2]=(3 4))"] {
        let (stderr, code) = rubash_n(script);
        assert!(
            stderr.contains("syntax error near unexpected token `('"),
            "{script}"
        );
        assert_eq!(code, Some(1), "{script}");
    }
}

/// Mid-word paren in an array element is rejected too (GNU lexes the
/// `(` as its own token).
#[test]
fn midword_paren_array_element_rejected() {
    let (stderr, code) = rubash_n("var=(x(y z))");
    assert!(stderr.contains("syntax error near unexpected token `('"));
    assert_eq!(code, Some(1));
}

/// Legal array shapes keep parsing: plain elements, subscripts, quoted
/// and escaped parens, arithmetic, process substitutions.
#[test]
fn legal_array_shapes_accepted() {
    for script in [
        "var=(1 2 3)",
        "var=([2]=x [0]=y)",
        "var=('(' paren)",
        "var=(\\( esc)",
        "var=($((1+2)) 9)",
        "var=(a >(echo x))",
    ] {
        let (stderr, code) = rubash_n(script);
        assert_eq!(stderr, "", "{script}");
        assert_eq!(code, Some(0), "{script}");
    }
}
