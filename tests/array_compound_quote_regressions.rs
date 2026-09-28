//! rubash#288: a `[sub]=value` compound array element must undergo exactly
//! ONE quote-removal pass, matching GNU's pipeline —
//! `expand_compound_array_assignment` (arrayfunc.c:557) runs
//! `parse_string_to_word_list` (tokenizes the raw body, quoting intact) and
//! then `expand_words_no_vars` (arrayfunc.c:610), after which
//! `assign_compound_array_list` stores `val = w + len + 2` (arrayfunc.c:839)
//! verbatim. The old rubash composition ran the storage unquote ON TOP of
//! `remove_shell_quotes`, re-reading literal backslashes as escape syntax:
//! `A=([0]='\[\e[33m\]Z')` stored `[e[33m]Z` and oh-my-bash agnoster printed
//! ` [e[33m]` noise in the prompt.
//!
//! Verified expectations come from WSL GNU Bash 5.3.0 (/usr/local/bin/bash)
//! script-file probes (`od -c` byte streams).

use std::process::Command;

fn rubash_raw(script: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    output.stdout
}

fn element_zero(script_head: &str) -> Vec<u8> {
    rubash_raw(&format!("{script_head}\nprintf '%s' \"${{V[0]}}\""))
}

const PROMPT_ESC: &[u8] = b"\\[\\e[33m\\]Z";

#[test]
fn single_quoted_subscript_element_keeps_backslashes() {
    assert_eq!(
        element_zero("V=([0]='\\[\\e[33m\\]Z')"),
        PROMPT_ESC,
        "parse.y:5305 read_token_word: '...' interior is literal"
    );
}

#[test]
fn declare_single_quoted_subscript_element_keeps_backslashes() {
    assert_eq!(
        element_zero("declare -a V=([0]='\\[\\e[33m\\]Z')"),
        PROMPT_ESC
    );
}

#[test]
fn double_quoted_subscript_element_keeps_non_cbsdquote_escapes() {
    // parse.y:5390-5393 + syntax.h:26 slashify_in_quotes = "\\`$\"\n":
    // `\e`/`\[`/`\]` inside "..." keep BOTH characters.
    assert_eq!(element_zero("V=([0]=\"\\[\\e[33m\\]Z\")"), PROMPT_ESC);
    assert_eq!(
        element_zero("declare -a V=([0]=\"\\[\\e[33m\\]Z\")"),
        PROMPT_ESC
    );
}

#[test]
fn dollar_single_quoted_subscript_element_decodes_once() {
    // GNU parse.y:5564-5571: ansiexpand's result is re-wrapped in
    // sh_single_quote, so the decoded bytes stay quoted through the one
    // removal. Expected bytes: \ [ ESC [ 3 3 m \ ] Z.
    assert_eq!(
        element_zero("V=([0]=$'\\[\\e[33m\\]Z')"),
        b"\\[\x1b[33m\\]Z"
    );
    // The `\]` unknown-escape pair keeps its backslash (GNU strtrans.c
    // ansicstr default branch) instead of collapsing to `]`.
    assert_eq!(element_zero("V=([0]=$'\\]')"), b"\\]");
}

#[test]
fn bare_subscript_element_still_drops_unquoted_backslashes() {
    // parse.y:5368-5397: an unquoted backslash quotes exactly the next
    // character, which survives WITHOUT the backslash.
    assert_eq!(element_zero("V=([0]=\\[\\eZ\\])"), b"[eZ]");
}

#[test]
fn append_subscript_element_keeps_backslashes() {
    assert_eq!(
        rubash_raw("V=(a)\nV+=('\\[\\e[44m\\]W')\nprintf '%s' \"${V[1]}\""),
        b"\\[\\e[44m\\]W"
    );
    assert_eq!(
        rubash_raw("V=(a)\nV+=([5]='\\[\\e[44m\\]W')\nprintf '%s' \"${V[5]}\""),
        b"\\[\\e[44m\\]W"
    );
}

#[test]
fn quotes_inside_double_quoted_element_are_data() {
    // One removal: the `'a'` inside "..." is data and must survive the
    // storage pass (a second removal used to strip it to `a`).
    assert_eq!(element_zero("V=([0]=\"'a'\")"), b"'a'");
    assert_eq!(element_zero("declare -a V=([0]=\"'a'\")"), b"'a'");
    // CBSDQUOTE: `\\` collapses to one backslash, exactly once.
    assert_eq!(element_zero("V=([0]=\"a\\\\b\")"), b"a\\b");
}

#[test]
fn agnoster_symbol_element_stays_glued_with_trailing_space() {
    // oh-my-bash agnoster stores prompt segments like '\[\e[33m\]bolt ' in
    // arrays: quote-span whitespace never field-splits.
    assert_eq!(
        rubash_raw("V=('\\[\\e[33m\\]bolt ')\nprintf '%s' \"${V[0]}\""),
        b"\\[\\e[33m\\]bolt "
    );
}

#[test]
fn mixed_quote_subscript_element_dequotes_positionally() {
    // 'it'\''s -> it's: escape + span adjacency, one removal.
    assert_eq!(element_zero("V=([0]='it'\\''s')"), b"it's");
    // a\'b (escaped quote outside quotes) -> a'b.
    assert_eq!(element_zero("V=([0]=a\\'b)"), b"a'b");
}

#[test]
fn scalar_append_keeps_expansion_result_backslashes() {
    // rubash#288 user-visible path: oh-my-bash agnoster builds its status
    // segment with `symbols+=$REPLY'⚡'` where REPLY holds a `\[\e[33m\]`
    // color prefix. GNU expand_string_assignment (subst.c:4345) dequotes
    // the word ONCE and variables.c's assign_array_element appends the
    // text to element 0 — expansion-produced backslashes are data (GNU
    // CTLESC-protects them via add_quoted_string, subst.c:11862 ->
    // quote_string, subst.c:4773). A second storage-layer unquote read
    // them as escape syntax and stored `[e[33m]⚡`.
    assert_eq!(
        rubash_raw(
            "REPLY='\\[\\e[33m\\]'\nsymbols=()\nsymbols+=$REPLY'⚡'\nprintf '%s' \"${symbols[0]}\"",
        ),
        b"\\[\\e[33m\\]\xe2\x9a\xa1"
    );
    // Double-quoted expansion + quoted suffix concatenates identically.
    assert_eq!(
        rubash_raw(
            "REPLY='\\[\\e[33m\\]'\nsymbols=()\nsymbols+=\"$REPLY\"'Q'\nprintf '%s' \"${symbols[0]}\"",
        ),
        b"\\[\\e[33m\\]Q"
    );
    // Appending to an existing element keeps both halves verbatim.
    assert_eq!(
        rubash_raw(
            "REPLY='\\[\\e[33m\\]'\nsymbols=(x)\nsymbols+=$REPLY'Q'\nprintf '%s' \"${symbols[0]}\"",
        ),
        b"x\\[\\e[33m\\]Q"
    );
    // `arr+=word` binds ONE scalar (variables.c assign_array_element): the
    // word never field-splits, matching `s=(a); s+="x y"` -> [0]="ax y".
    assert_eq!(
        rubash_raw("s=(a)\ns+=\"x y\"\nprintf '%s' \"${s[0]}\""),
        b"ax y"
    );
}
