//! rubash#733: quote state must not leak between compound-assignment
//! elements. GNU arrayfunc.c:581 parse_string_to_word_list re-parses the
//! compound body with the shell parser, so each element word gets its own
//! quoting context (parse.y:5305 read_token_word tracks `'`/`"` per word).
//!
//! Two root causes fixed here:
//! 1. `hoist_data_single_quotes` ran after `hoist_data_double_quotes` but
//!    did not track the \u{E302} double-quote sentinel: the `'` in
//!    `("it's" '$h')` opened a single-quote span that closed at the `'`
//!    before `$h`, leaving `$h` unquoted (and swallowing the `"`-quoted
//!    `$h` in the `"$h"` variant into a never-closed span).
//! 2. `expand_unquoted_parameter_compound_assignment` stripped a
//!    single-quoted whole-word element and expanded it as a parameter:
//!    `declare d1=('$h')` stored the value of `h`.
//!
//! Verified expectations come from the issue's GNU Bash 5.3.0 probes.

use std::process::Command;

fn rubash(script: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "rubash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8")
}

#[test]
fn apostrophe_element_does_not_flip_next_single_quoted_element() {
    // GNU: [$h] — the '...' element stays literal.
    assert_eq!(
        rubash("h=X; b1=(\"it's\" '$h'); printf '[%s]' \"${b1[1]}\""),
        "[$h]"
    );
}

#[test]
fn apostrophe_element_does_not_flip_next_double_quoted_element() {
    // GNU: [X] — the "..." element still expands.
    assert_eq!(
        rubash("h=X; b2=(\"it's\" \"$h\"); printf '[%s]' \"${b2[1]}\""),
        "[X]"
    );
}

#[test]
fn declare_single_quoted_compound_element_stays_literal() {
    // GNU: [$h]
    assert_eq!(
        rubash("h=X; declare d1=('$h'); printf '[%s]' \"${d1[0]}\""),
        "[$h]"
    );
}

#[test]
fn declare_double_quoted_compound_element_still_expands() {
    // GNU: [X] — double quotes are unaffected by the single-quote fix.
    assert_eq!(
        rubash("h=X; declare d1=(\"$h\"); printf '[%s]' \"${d1[0]}\""),
        "[X]"
    );
}

#[test]
fn declare_unquoted_compound_element_still_expands() {
    // GNU: [X]
    assert_eq!(
        rubash("h=X; declare d1=($h); printf '[%s]' \"${d1[0]}\""),
        "[X]"
    );
}

#[test]
fn declare_single_quoted_star_does_not_glob() {
    // GNU: n=1 e0=* — a quoted '*' never undergoes pathname expansion.
    let out = rubash(
        "mkdir -p /tmp/rubash-733-glob && cd /tmp/rubash-733-glob && : > aa.txt; \
         declare d2=('*'); printf 'n=%s e0=%s' \"${#d2[@]}\" \"${d2[0]}\"",
    );
    assert_eq!(out, "n=1 e0=*");
}

#[test]
fn mixed_quoted_elements_keep_their_own_state() {
    // GNU: [a b][X][c] — each element's quoting is independent.
    assert_eq!(
        rubash(
            "h=X; i=('a b' \"$h\" c); printf '[%s][%s][%s]' \"${i[0]}\" \"${i[1]}\" \"${i[2]}\""
        ),
        "[a b][X][c]"
    );
}
