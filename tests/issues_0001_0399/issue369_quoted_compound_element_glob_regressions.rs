//! rubash#369: a compound array element that is ENTIRELY a quoted
//! parameter expansion (`arr=("$x")`) must keep its expansion result as
//! DATA — pathname expansion requires an UNQUOTED glob character in the
//! word (GNU `pathexp.c:57 unquoted_glob_pattern_p`, whose `case CTLESC`
//! at pathexp.c:120-122 skips the protected characters).
//!
//! GNU pipeline: `expand_compound_array_assignment` (arrayfunc.c:557)
//! tokenizes the body with `parse_string_to_word_list` (arrayfunc.c:575,
//! quoting intact, W_QUOTED per word) and expands each word via
//! `expand_words_no_vars` (arrayfunc.c:605 -> subst.c:12590
//! expand_word_list_internal). Inside `expand_word_internal`
//! (subst.c:11229) a quoted `$param` result goes through
//! `add_quoted_string` (subst.c:11862) -> `quote_string` (subst.c:4773),
//! so EVERY character arrives at the glob gate CTLESC-protected;
//! `glob_expand_word_list` (subst.c:12602) then sees no unquoted pattern
//! character and `dequote_string` (subst.c:4807) strips the pairs for
//! storage. The UNQUOTED element skips the protection, so its fields DO
//! glob.
//!
//! Root cause on the rubash side: the whole-word parameter arm
//! (`whole_word_parameter_compound_fields`, introduced by 306436d7)
//! transported the quoted product as `ARRAY_FIELD_SPLIT_MARKER +
//! quote_array_value(value)` — the transport form of UNQUOTED field-split
//! products, whose quotes are storage serialization DATA. The marker arm
//! of `append_array_value` pathname-expands those tokens AFTER
//! `unquote_storage_value` strips the wrapper, so the quoting state was
//! gone and `x='*'; arr=("$x")` stored the glob matches. The fix returns
//! the plain `"..."` storage word (the `store!` product form), whose
//! quote span `compound_element_glob_pattern` treats as syntax.
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash) script-file probes.

use std::process::Command;

fn rubash(script: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    output.stdout
}

/// Elements rendered as `[%s]` joined, the m369 matrix form.
fn elements(script_head: &str) -> Vec<u8> {
    rubash(&format!(
        "{script_head}\nprintf '[%s]' \"${{arr[@]}}\"; echo",
    ))
}

#[test]
fn quoted_whole_word_parameter_element_stays_literal() {
    // The #369 reproducer: GNU `[*][literal]`, the bug stored the matches.
    assert_eq!(
        elements("x='*'; arr=(\"$x\" literal)"),
        b"[*][literal]\n",
        "pathexp.c:57: a quoted `*` is CTLESC-protected data, not a pattern"
    );
}

#[test]
fn unquoted_whole_word_parameter_element_still_globs() {
    // Control (must NOT be over-suppressed): unquoted `$x` globs.
    // The fixture lives in the Rust temp dir (path passed in, so the test
    // does not depend on the shell's /tmp mapping).
    let dir = std::env::temp_dir().join("rubash369ctl");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir fixture");
    std::fs::write(dir.join("only369"), b"").expect("touch fixture");
    let script = format!(
        "cd '{}'; x='only*'; arr=($x); printf '[%s]' \"${{arr[@]}}\"; echo",
        dir.display()
    );
    let out = rubash(&script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out, b"[only369]\n",
        "unquoted expansion fields are plain pattern words (subst.c:12602)"
    );
}

#[test]
fn quoted_append_compound_element_stays_literal() {
    assert_eq!(
        elements("x='*'; arr=(p1 p2); arr+=(\"$x\")"),
        b"[p1][p2][*]\n",
        "quoted append element carries the same W_QUOTED no-glob contract"
    );
}

#[test]
fn quoted_whole_word_keeps_question_and_bracket_literals() {
    assert_eq!(
        elements("x='*'; arr=(\"$x\" '?')"),
        b"[*][?]\n",
        "a quoted `?` element is data (pathexp.c unquoted_glob_pattern_p)"
    );
    assert_eq!(
        elements("x='*'; arr=(\"$x\" '[ab]')"),
        b"[*][[ab]]\n",
        "a quoted bracket expression element is data"
    );
}

#[test]
fn quoted_at_list_fanout_elements_stay_literal() {
    // `store!` product control: the `"${a[@]}"` fan-out already rode the
    // plain quoted-word transport and must keep working.
    assert_eq!(
        elements("a=('*' 'x y'); arr=(\"${a[@]}\")"),
        b"[*][x y]\n",
        "each quoted at-list member is one CTLESC-protected element"
    );
}

#[test]
fn partial_quote_shapes_unchanged() {
    // Adjacent-quote concatenations: quoted span suppresses the glob char.
    assert_eq!(
        elements("x='*'; arr=(\"$x\"suf)"),
        b"[*suf]\n",
        "the quoted span protects only its own characters"
    );
    assert_eq!(
        elements("x='*'; arr=(pre\"$x\")"),
        b"[pre*]\n",
        "an unquoted literal prefix plus quoted expansion stays one element"
    );
    assert_eq!(
        elements("arr=('*.lit')"),
        b"[*.lit]\n",
        "single-quoted literals never glob (parse.y read_token_word)"
    );
}

#[test]
fn quoted_empty_and_whitespace_values_stay_one_element() {
    assert_eq!(
        rubash("unset v; arr=(\"$v\"); printf 'count=%d' \"${#arr[@]}\"; echo"),
        b"count=1\n",
        "quoted null stores ONE empty element (QUOTED_NULL, subst.c:11940)"
    );
    assert_eq!(
        elements("v='a b'; arr=(\"$v\")"),
        b"[a b]\n",
        "W_QUOTED suppresses field splitting (subst.c:12050 W_NOSPLIT arm)"
    );
}
