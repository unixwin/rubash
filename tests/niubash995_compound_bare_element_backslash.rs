//! niubash#995: a quoted `(...)` compound element of a declaration builtin
//! lost its backslashes — `local -a d=("$1" "$@")` with a parameter holding
//! `a\b` stored `ab`.
//!
//! Root cause is the storage unquote, not the expansion: the element reaches
//! `unquote_storage_value` as a whole-token sh_double_quote serialization
//! (`"<bytes>"`, exactly what arrays/storage.rs `quote_array_value` emits),
//! and the old arm decoded EVERY `\x` pair. GNU's shquote.c:134
//! sh_double_quote escapes only slashify_in_quotes (syntax.h:26) — `\`, `$`,
//! `"` and `` ` `` — so a backslash before anything else is data and both
//! characters must survive. Only the plain `d=("a\b")` assignment was
//! correct because it binds through the executor's `append_array_value`
//! (which re-quotes with `quote_array_value` first); every declaration
//! builtin (`declare`/`typeset`/`local`/`readonly`/`export`) took the
//! storage-unquote arm and ate the backslash.
//!
//! Expectations verified against GNU Bash 5.3.9 (Git for Windows) and
//! 5.3.20 script-file probes; the sibling file
//! `array_compound_quote_regressions.rs` (rubash#288) covers the
//! `[sub]=value` element form of the same quote-removal pipeline.

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

/// Value of `V[0]` after running `decl` plus a bare-element print.
fn element0(decl: &str) -> String {
    run(&format!("{decl}\nprintf '%s' \"${{V[0]}}\""))
}

#[test]
fn declared_single_quoted_bare_element_keeps_backslashes() {
    // parse.y:5305 read_token_word: a '...' interior is literal, so `\b`
    // is two characters and the storage unquote must not touch it.
    assert_eq!(element0("declare -a V=('a\\b')"), "a\\b");
}

#[test]
fn declared_double_quoted_bare_element_keeps_non_escape_backslashes() {
    // `b` is not one of slashify_in_quotes, so both characters survive.
    assert_eq!(element0("declare -a V=(\"a\\b\")"), "a\\b");
}

#[test]
fn declared_double_quoted_doubled_backslash_collapses_to_one() {
    // `\\` IS a slashify escape: the pair collapses to a single backslash.
    assert_eq!(element0("declare -a V=(\"a\\\\b\")"), "a\\b");
}

#[test]
fn declared_bare_unquoted_element_still_decodes_its_escape() {
    // Control: an UNQUOTED `\b` is escape syntax and decodes to `b`. The fix
    // must not turn this into a literal backslash.
    assert_eq!(element0("declare -a V=(a\\b)"), "ab");
}

#[test]
fn the_four_slashify_escapes_still_decode_inside_quotes() {
    // shquote.c:134 escapes exactly these four, so the storage unquote must
    // keep decoding them.
    assert_eq!(
        element0("declare -a V=(\"a\\\\b\")"),
        "a\\b",
        "`\\\\` -> `\\`"
    );
    assert_eq!(
        element0("declare -a V=(\"a\\\"b\")"),
        "a\"b",
        "`\\\"` -> `\"`"
    );
    assert_eq!(element0("declare -a V=(\"a\\$b\")"), "a$b", "`\\$` -> `$`");
    assert_eq!(
        element0("declare -a V=(\"a\\`b\")"),
        "a`b",
        "`\\`` -> `` ` ``"
    );
}

#[test]
fn local_array_keeps_backslashes() {
    // The issue as reported: `local -a` inside a function.
    assert_eq!(
        run("f() { local -a V=('a\\b' \"c\\d\"); printf '%s|%s' \"${V[0]}\" \"${V[1]}\"; }\nf"),
        "a\\b|c\\d"
    );
}

#[test]
fn local_array_from_positional_parameters_keeps_backslashes() {
    // The exact #995 reproducer shape: parameters expanded into a compound
    // literal must keep their backslashes.
    assert_eq!(
        run(
            "f() {\n  local first=\"$1\"; shift\n  local -a drop=(\"$first\" \"$@\")\n  \
             printf '%s|%s' \"${drop[0]}\" \"${drop[1]}\"\n}\nf 'a\\b' 'c\\d'"
        ),
        "a\\b|c\\d"
    );
}

#[test]
fn assoc_bare_and_subscript_elements_keep_backslashes() {
    assert_eq!(
        run("declare -A V=([k]=\"a\\b\"); printf '%s' \"${V[k]}\""),
        "a\\b"
    );
    assert_eq!(
        run("declare -A V=([k]='p\\q'); printf '%s' \"${V[k]}\""),
        "p\\q"
    );
}

#[test]
fn mixed_quoted_elements_keep_backslashes_and_spaces() {
    // A quoted element with whitespace must stay ONE element while its
    // backslash survives.
    assert_eq!(
        run("declare -a V=(\"a\\b\" \"c d\" 'e\\f'); printf '%s|%s|%s' \
             \"${V[0]}\" \"${V[1]}\" \"${V[2]}\""),
        "a\\b|c d|e\\f"
    );
}

#[test]
fn plain_assignment_control_is_unchanged() {
    // The path that was already correct must stay correct.
    assert_eq!(element0("V=('a\\b')"), "a\\b");
    assert_eq!(element0("V=(\"a\\b\")"), "a\\b");
    assert_eq!(element0("V=(a\\b)"), "ab");
}
