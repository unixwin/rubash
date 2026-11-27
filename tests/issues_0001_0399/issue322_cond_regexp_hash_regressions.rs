//! Issue rubash#322: `[[ str =~ ^(a|b)$ ]]` with an unquoted parenthesized
//! regex aborted the parse ("unexpected end of file from `('"), killing
//! pbb's is_hex_color section (003) end-to-end. The trigger was narrower
//! than "any unquoted paren regex": a `#` inside the regex (e.g. the
//! `#?` hex-color prefix `^(#?([a-fA-F0-9]{6}|[a-fA-F0-9]{3}))$`) hit the
//! tokenizer's word-initial `#` comment rule when the regex was split at
//! the `(` token boundary — GNU never re-lexes there.
//!
//! GNU semantics (vendored third_party/bash):
//! - parse.y:5169: after the `=~` operator word, parser_state |= PST_REGEXP
//!   (parser.h:38) for exactly the RHS word; parse.y:5212 clears it.
//! - parse.y:5443-5461 (read_token_word under PST_REGEXP): `|` is a plain
//!   word character and an unquoted `(` consumes its balanced `(...)` group
//!   verbatim via parse_matched_pair — the whole regex is ONE word, so a
//!   `#` inside it (at ANY position, including after whitespace inside the
//!   group: `[[ "(a #c)" =~ (a #c) ]]`) is word DATA. read_token's `#`
//!   comment branch (parse.y:3607) only fires at a word START — so
//!   `[[ a =~ #x ]]` still comments out the rest of the line (GNU rc 2).
//!
//! Fix: src/lexer/scanner.rs ports PST_CONDCMD/PST_REGEXP into
//! LexerParseState (record_token) and the `#` branch keeps a token-initial
//! `#` as word data while the RHS word is open (first fragment consumed,
//! and either inside a `(` group or directly abutting the previous
//! fragment).
//!
//! KNOWN REMAINING GAP (captain-owned file): when the LHS is a QUOTED
//! literal (`[[ "#FFFFFF" =~ ^(#?...)$ ]]`), the script driver's
//! completeness oracle (lexer/continuation.rs, captain-exclusive) still
//! reports a phantom unterminated `(`: its at_command tracker does not see
//! a completed quoted span as word material. Validated proposed diff:
//! target/issue-suites/results/pbbfix/issue322-continuation-proposed.diff.
//! These tests cover the class that works without it (variable/unquoted
//! LHS — the shape pbb's is_hex_color function actually uses with `$1`).
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

/// The issue's regex shape with a variable LHS (the pbb is_hex_color
/// function form): parses and matches with BASH_REMATCH populated.
#[test]
fn unquoted_paren_regex_with_hash_parses_and_matches() {
    let (stdout, stderr, code) = rubash(
        "v=\"#FFFFFF\"\n\
         if [[ $v =~ ^(#?([a-fA-F0-9]{6}|[a-fA-F0-9]{3}))$ ]]; then\n\
         echo \"match:${BASH_REMATCH[1]}\"\n\
         else\n\
         echo nomatch\n\
         fi",
    );
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "match:#FFFFFF\n");
}

/// pbb is_hex_color end-to-end (section 003) with `$1` LHS.
#[test]
fn pbb_is_hex_color_end_to_end() {
    let (stdout, stderr, _) = rubash(
        "is_hex_color() {\n\
         if [[ $1 =~ ^(#?([a-fA-F0-9]{6}|[a-fA-F0-9]{3}))$ ]]; then\n\
         printf '%s\\n' \"$1 is valid.\"\n\
         else\n\
         printf '%s\\n' \"$1 is not valid.\"\n\
         fi\n\
         }\n\
         is_hex_color \"#FFFFFF\"\n\
         is_hex_color \"123456\"\n\
         is_hex_color \"12345\"\n\
         is_hex_color \"invalid\"",
    );
    assert_clean(&stderr);
    assert_eq!(
        stdout,
        "#FFFFFF is valid.\n123456 is valid.\n12345 is not valid.\ninvalid is not valid.\n"
    );
}

/// A match using the `#?` prefix: `$s="#ab"` matches `^(#?(ab))$` (the
/// optional-# arm is live, not commented away).
#[test]
fn hash_optional_prefix_regex_matches() {
    let (stdout, stderr, code) = rubash("s=\"#ab\"\n[[ $s =~ ^(#?(ab))$ ]] && echo M");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "M\n");
}

/// `#` directly inside the regex paren group stays data (this is the exact
/// fragment the old lexer commented out) — both the match and non-match
/// arms verify the regex engine saw the full pattern.
#[test]
fn hash_after_open_paren_in_regex_is_data() {
    let (stdout, stderr, code) = rubash(
        "y=ac\n[[ $y =~ ^(#?a)c$ ]] && echo M1 || echo N1\n[[ $y =~ ^(#?b)c$ ]] && echo M2 || echo N2",
    );
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "M1\nN2\n");
}

/// A `#` directly abutting a closed group at depth 0 stays data
/// (`[[ "(b)#c" =~ (b)#c ]]` — GNU reads the RHS as one word `(b)#c`).
#[test]
fn hash_abutting_closed_group_is_data() {
    let (stdout, stderr, code) = rubash("s=b#c\n[[ $s =~ (b)#c ]] && echo abut");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "abut\n");
}

/// GNU still treats a word-initial `#` after `=~` as a comment (the RHS
/// word never started): rc 2 with the conditional-binary-operator error —
/// the fix must NOT swallow this class.
#[test]
fn word_initial_hash_after_regex_op_is_still_a_comment() {
    let (_, stderr, code) = rubash("[[ y =~ #?(x) ]] && echo c");
    assert_eq!(code, Some(2));
    assert!(
        stderr.contains("unexpected argument") && stderr.contains("conditional"),
        "stderr: {stderr}"
    );
}

/// Comments after a complete conditional are unaffected.
#[test]
fn comment_after_conditional_still_works() {
    let (stdout, stderr, code) = rubash("[[ y =~ (b) ]] # trailing comment\necho after");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
}

/// Ordinary `#`-comment behavior outside conditionals is unchanged.
#[test]
fn plain_comments_unchanged() {
    let (stdout, stderr, code) = rubash("echo one # comment\necho x#y # c");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "one\nx#y\n");
}

/// Variable regex via `$re` (pbb regex() helper shape) keeps working.
#[test]
fn variable_regex_helper_still_works() {
    let (stdout, stderr, code) = rubash(
        "regex() { [[ $1 =~ $2 ]] && echo match || echo no; }\n\
         re='^(a|b)$'\n\
         regex b $re\n\
         regex c $re",
    );
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "match\nno\n");
}
