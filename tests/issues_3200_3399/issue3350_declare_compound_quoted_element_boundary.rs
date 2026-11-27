//! rubash#3350: a `declare` compound element that is single-quoted and
//! contains a `"` merged with every following element —
//! `declare -a a=('a"b' 'c'); echo ${#a[@]}` stored ONE element
//! (`$'a"b" \020"c'`, the `\x10` element-boundary marker leaking into the
//! value) where GNU stores two.
//!
//! Root cause is the storage word SPLIT, not the decode: the declare
//! operand path (`expand_declare_compound_elements`, executor/
//! subscript_expansion.rs) serializes each literal element as a
//! `\x10"..."` transport word whose interior quotes are CTLESC-protected
//! (`compound_element_glob_pattern`: `'a"b'` -> `\x11a\x11"\x11b`, wrapped
//! as `"\x11a\x11"\x11b"`). The storage splitter (`split_storage_words`,
//! both the declare twin in builtins/declare/storage/words.rs and the
//! executor twin in executor/assignment_helpers.rs) did not treat a
//! CTLESC pair as protected data: the bare `"` inside the wrapper toggled
//! its double-quote state, so the space between elements read as
//! inside-quotes and the whole rest of the list became one word. A quoted
//! space inside the element split it mid-word instead (`'a"b c'` ->
//! three garbled elements).
//!
//! GNU ground (third_party/bash, Bash-5.3 patch 20): `quote_string`
//! (subst.c:4773) prefixes EVERY character of a quoted string with
//! CTLESC, so a protected character is data, never syntax; `dequote_string`
//! (subst.c:4807) strips the pairs at the end. The compound list itself is
//! re-parsed with real quoting (`parse_string_to_word_list`,
//! arrayfunc.c:581, called from `expand_compound_array_assignment`,
//! arrayfunc.c:557) and expanded per word (`expand_words_no_vars`,
//! arrayfunc.c:610). The fix teaches both `split_storage_words` twins to
//! glue a CTLESC pair exactly like the IFS_GLUE / COMPOUND_EXPANSION_WS_TAG
//! pairs they already glue: the protected character can neither toggle
//! quote state nor act as a split boundary.
//!
//! Expectations verified against GNU Bash 5.3.20 (issue oracle) and
//! re-checked against local GNU 5.2.21 (same counts/values for these
//! shapes). The plain-assignment form `a=('a"b' 'c')` was already correct
//! and is pinned as a control; `local -a` in a function shares the
//! declare operand path and is pinned too; the assoc `['a"b']=1` form was
//! already correct and is pinned as a control.

use std::process::Command;

fn run(script: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "rubash failed for {script:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Element count and `|`-joined element values of array `V` after `decl`.
fn describe(decl: &str) -> String {
    run(&format!(
        "{decl}\nprintf '%s|%s' \"${{#V[@]}}\" \"${{V[*]}}\""
    ))
}

#[test]
fn issue3350_a1_single_quoted_element_holding_dquote_splits() {
    // GNU: 2 elements, `a"b` and `c`. Base rubash: 1 element, `a"b" \x10"c`.
    assert_eq!(
        describe("declare -a V=('a\"b' 'c')"),
        "2|a\"b c",
        "A1: ('a\"b' 'c')"
    );
}

#[test]
fn issue3350_a2_dquote_at_element_end_splits() {
    // Same mechanism, quote position differs (issue A2).
    assert_eq!(
        describe("declare -a V=('ab\"' 'c')"),
        "2|ab\" c",
        "A2: ('ab\"' 'c')"
    );
}

#[test]
fn issue3350_a4_quoted_space_inside_element_stays_glued() {
    // The element's own quoted space must not split it mid-word, and the
    // following element must stay separate. Base rubash: 3 garbled
    // elements (`ab`, `c`, `\x10d`).
    assert_eq!(
        describe("declare -a V=('a\"b c' 'd')"),
        "2|a\"b c d",
        "A4: ('a\"b c' 'd')"
    );
}

#[test]
fn issue3350_a5_two_quoted_elements_split() {
    assert_eq!(
        describe("declare -a V=('a\"b' 'c\"d' 'e')"),
        "3|a\"b c\"d e",
        "A5: ('a\"b' 'c\"d' 'e')"
    );
}

#[test]
fn issue3350_a3_control_first_element_without_dquote() {
    // Control (issue A3): no `"` in the first element — already green.
    assert_eq!(
        describe("declare -a V=('ab' 'c')"),
        "2|ab c",
        "A3: ('ab' 'c')"
    );
}

#[test]
fn issue3350_plain_assignment_control() {
    // Control: the plain assignment form never took the declare operand
    // path and was already correct.
    assert_eq!(
        describe("V=('a\"b' 'c')"),
        "2|a\"b c",
        "plain V=('a\"b' 'c')"
    );
}

#[test]
fn issue3350_double_quoted_element_holding_squote_control() {
    // Control: a double-quoted first element holding `'` was already fine.
    assert_eq!(
        describe("declare -a V=(\"a'b\" 'c')"),
        "2|a'b c",
        "(\"a'b\" 'c')"
    );
}

#[test]
fn issue3350_local_in_function_shares_declare_path() {
    let out = run("f() { local -a V=('a\"b' 'c'); printf '%s|%s' \"${#V[@]}\" \"${V[*]}\"; }; f");
    assert_eq!(out, "2|a\"b c", "local -a inside a function");
}

#[test]
fn issue3350_assoc_subscript_form_control() {
    // Control: the assoc `[key]=value` form was already correct.
    assert_eq!(
        run("declare -A M=(['a\"b']=1 ['c']=2); printf '%s|%s|%s' \"${#M[@]}\" \"${M['a\"b']}\" \"${M['c']}\""),
        "2|1|2",
        "assoc (['a\"b']=1 ['c']=2)"
    );
}

#[test]
fn issue3350_a2_delim_backslash_ending_element_splits() {
    // Delimitation original A2 (#3346 probe-adjacent.sh, verbatim shape —
    // NOT the a2 cell above, whose quote sits mid-element): the first
    // element ends in a backslash. GNU: 2 elements, `a\` and `b`.
    // Base rubash: 1 merged element with the \x10 marker leaking.
    assert_eq!(
        describe("declare -a V=('a\\' 'b')"),
        "2|a\\ b",
        "A2-delim: ('a\\' 'b')"
    );
}

#[test]
fn issue3350_a4_delim_double_quoted_escaped_quote_splits() {
    // Delimitation original A4 (#3346 probe-adjacent.sh, verbatim shape —
    // NOT the a4 cell above, whose quoted space sits inside a
    // single-quoted element): a double-quoted element holding an escaped
    // quote. GNU: 2 elements, `a"b` and `c`. Base rubash: 1 merged
    // element with the \x10 marker leaking.
    assert_eq!(
        describe("declare -a V=(\"a\\\"b\" 'c')"),
        "2|a\"b c",
        "A4-delim: (\"a\\\"b\" 'c')"
    );
}
