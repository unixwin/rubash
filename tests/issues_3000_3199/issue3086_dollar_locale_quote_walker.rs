//! rubash#3086 — `$"..."` is not decoded as a locale string inside
//! `$((...))` bodies or `${...}` operator words: the shared embedded-
//! parameter walker has no `$"` arm, so it emits the `$` literally and
//! lets the `"` toggle quote state (inside a default word, `$a` then
//! expands as a variable and the word collapses to ` b`). In the C
//! locale GNU treats `$"..."` exactly like `"..."` in every word
//! context (subst.c:11229 `expand_word_internal`; the `$"` translation
//! arm at subst.c:1606/1639), and arithmetic bodies and operator words
//! both flow through that same scanner. The lexer (ordinary words —
//! D7 control) and the array-subscript converters (rubash#3023)
//! already handle `$"`; only the walker misses it.
//!
//! Expectations byte-measured against GNU bash 5.3.20: every cell is
//! rc 0 with empty stderr. Harness mirrors issue3024: `rubash -c` via
//! `CARGO_BIN_EXE_rubash`, `LC_ALL=C.UTF-8` (the C locale face: no
//! translation catalog, `$"` ≡ `"`).

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

fn expect_value(script: &str, expected_stdout: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.code,
        0,
        "{ctx}: rc must be 0; stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout,
        expected_stdout,
        "{ctx}: stdout mismatch; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn arith_body_locale_literal_evaluates() {
    expect_value("echo $(( $\"1+2\" ))", b"3\n", "D1 $(( $\"1+2\" ))");
}

#[test]
fn arith_body_locale_variable_set() {
    expect_value("x=4; echo $(( $\"x\" ))", b"4\n", "D2 $(( $\"x\" )), x set");
}

#[test]
fn arith_body_locale_variable_unset() {
    expect_value(
        "unset x; echo $(( $\"x\" ))",
        b"0\n",
        "D3 $(( $\"x\" )), x unset",
    );
}

#[test]
fn default_word_locale_string_keeps_whole_word() {
    expect_value(
        "unset q; echo \"X${q-$\"a b\"}Y\"",
        b"Xa bY\n",
        "D4 ${q-$\"a b\"}",
    );
}

#[test]
fn colon_default_word_locale_string() {
    expect_value(
        "q=; echo \"X${q:-$\"a b\"}Y\"",
        b"Xa bY\n",
        "D5 ${q:-$\"a b\"}",
    );
}

#[test]
fn arith_body_locale_expression_with_spaces() {
    expect_value(
        "x=4; echo $(( $\"x + 1\" ))",
        b"5\n",
        "D6 $(( $\"x + 1\" ))",
    );
}

#[test]
fn plain_word_locale_string_is_control() {
    // Control: ordinary words go through the lexer, which already
    // decodes $"..." — this cell guards the walker's fix against
    // double-decoding the already-correct face.
    expect_value("echo $\"a b\"", b"a b\n", "D7 plain $\"a b\" (control)");
}

#[test]
fn arith_locale_chain_across_commands() {
    // Chained: the first arithmetic result feeds the second expansion.
    expect_value(
        "v=$(( $\"2\" + 3 )); echo \"v=$v\"; echo $(( v * $\"2\" ))",
        b"v=5\n10\n",
        "D8 chained $((...)) with $\" operands",
    );
}

#[test]
fn assign_default_word_locale_string_chain() {
    // Chained state assertion: the := word both expands and assigns
    // the decoded value, read back on the next line.
    expect_value(
        "unset q; echo \"X${q:=$\"a b\"}Y\"; echo \"q=[$q]\"",
        b"Xa bY\nq=[a b]\n",
        "D9 ${q:=$\"a b\"} then read q",
    );
}

#[test]
fn plus_word_locale_string() {
    expect_value(
        "q=set; echo \"X${q+$\"a b\"}Y\"",
        b"Xa bY\n",
        "D10 ${q+$\"a b\"}",
    );
}

#[test]
fn default_word_locale_inside_arith_body() {
    // Chained contexts: a ${...} default word inside an arithmetic
    // body, its word itself a locale string.
    expect_value(
        "unset q; echo $(( ${q-$\"2\"} + 1 ))",
        b"3\n",
        "D11 $(( ${q-$\"2\"} + 1 ))",
    );
}

#[test]
fn default_word_dollar_single_quote_is_control() {
    // Control: the walker's $'...' arm already exists — the same word
    // shape with ANSI-C quoting expands correctly today. The defect is
    // specifically the missing $" arm next to it.
    expect_value(
        "unset q; echo \"X${q-$'a b'}Y\"",
        b"Xa bY\n",
        "D12 ${q-$'a b'} (control)",
    );
}
