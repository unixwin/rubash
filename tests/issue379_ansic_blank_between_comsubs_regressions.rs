//! rubash#379: a `$'...'` unit between two comsubs in one word was folded
//! to a space — `echo A$((1+1))B$'\n'C$((2+2))D` printed `A2B C4D` (one
//! line) instead of GNU's two lines.
//!
//! GNU spec: parse.y:5560-5575 hands every ANSI-C decoded byte to
//! read_token_word as QUOTED characters (ansiexpand + the CTLESC re-encode
//! of sh_single_quote), so a decoded blank is literal WORD DATA; subst.c
//! field splitting only ever runs on EXPANSION RESULTS
//! (expand_word_internal splits each substitution's output), never on
//! source-word bytes. The corruption was the split machinery treating the
//! decoded blank as an expansion-result separator once the word's arith/
//! command-substitution spans made it take the field-split path.
//!
//! Fix (one invariant, four boundaries):
//! - lexer/quotes.rs `escape_decoded_ansi_c_quotes` — the single decode
//!   choke point both `$'...'` lex arms use — now marks decoded blanks
//!   with ANSI_C_IFS_GUARD (\u{E201}, markers.rs: a PUA codepoint OUTSIDE
//!   the raw-byte pair payload range E001..=E100 and the byte-char range
//!   E100..=E1FF, because comsub payload protection encodes every C0
//!   byte and would capture a C0 carrier mid-transport) and CTLESC-tags
//!   decoded glob metacharacters (`* ? [`, extglob introducers), the
//!   add_quoted_string model of subst.c:11862.
//! - Field-split call sites convert the guard to IFS_GLUE (the splitters'
//!   existing "next char is literal" carrier): command_words
//!   `field_split_values`, command_prepare's unquoted-expanded-word split.
//! - Value boundaries drop the guard: argv materialization
//!   (materialize_expanded_command_word), the assignment funnel
//!   (expand_assignment_value), for-list values, redirect operands,
//!   conditional words.
//! - `raw_word_is_quoted` treats `$'...'` as a quoted SEGMENT (skip-span
//!   like `${...}`/`$(...)`, parse.y:5546-5558 P_ALLOWESC), not outer
//!   quoting of the whole word — `$c$'\n'g` still field-splits $c's
//!   result at IFS whitespace. `redirect_target_is_ambiguous` skips
//!   `$'`/`$"` spans the same way.
//!
//! Expectations byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash) script-file probes (target/probe379, 2026-10-02).

use std::process::Command;

fn rubash(script: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    output.stdout
}

#[test]
fn ansic_newline_between_two_arith_comsubs() {
    // The #379 reproducer: GNU prints two lines.
    assert_eq!(rubash("echo A$((1+1))B$'\\n'C$((2+2))D"), b"A2B\nC4D\n");
}

#[test]
fn ansic_tab_and_space_between_two_arith_comsubs() {
    assert_eq!(
        rubash("printf '%s' A$((1+1))B$'\\t'C$((2+2))D; echo"),
        b"A2B\tC4D\n",
        "a decoded tab is word data (byte 0x09, not a separator)"
    );
    assert_eq!(
        rubash("echo A$((1+1))B$' 'C$((2+2))D"),
        b"A2B C4D\n",
        "a decoded space stays inside the one word"
    );
}

#[test]
fn ansic_unit_between_command_substitutions() {
    assert_eq!(
        rubash("echo X$(echo u)v$'\\n'w$(echo x)y"),
        b"Xuv\nwxy\n",
        "same invariant with $(...) spans"
    );
}

#[test]
fn ansic_unit_adjacent_to_one_comsub() {
    assert_eq!(rubash("echo P$((4+4))Q$'\\n'R"), b"P8Q\nR\n");
    assert_eq!(rubash("echo P$'\\n'Q$((4+4))R"), b"P\nQ8R\n");
}

#[test]
fn param_result_splits_but_decoded_blank_does_not() {
    // `$c`'s unquoted RESULT field-splits (GNU: e | f\n-g); the decoded
    // newline between f and g is literal word data. Byte-verified vs GNU
    // (target/probe379/t5-t7): output is two lines `e f` and `g`.
    assert_eq!(
        rubash("c=$(printf 'e\\nf'); printf '[%s]' $c$'\\n'g; echo"),
        b"[e][f\ng]\n",
        "raw_word_is_quoted: $'...' is a quoted segment, not word quoting"
    );
}

#[test]
fn assignment_positions_store_clean_values() {
    // The assignment funnel must strip the guard: the stored value is real
    // text with the correct length (the guard byte never reaches a cell).
    assert_eq!(
        rubash("v=A$((1+1))B$'\\n'C$((2+2))D; printf 'v=[%s] len=%d' \"$v\" \"${#v}\"; echo"),
        b"v=[A2B\nC4D] len=7\n",
        "assignment value keeps the decoded newline, byte count 7"
    );
    assert_eq!(
        rubash("v2=$'p\\nq'$(echo r)s; printf 'len=%d' \"${#v2}\"; echo"),
        b"len=5\n",
        "comsub-bearing assignment words store clean values too"
    );
    assert_eq!(
        rubash("v3=$'a b'; printf 'len=%d' \"${#v3}\"; echo"),
        b"len=3\n",
        "plain decoded blank in an assignment value"
    );
}

#[test]
fn decoded_glob_metachars_stay_literal() {
    // pathexp.c:57: decoded metacharacters are CTLESC-protected data, so
    // `echo $'a*b'` never globs even when a match exists; the STORED
    // value's `*` is ordinary data that DOES glob on later unquoted
    // expansion (dequote_ctlesc_pairs runs at the assignment boundary).
    let dir = std::env::temp_dir().join("rubash379fx");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir fixture");
    std::fs::write(dir.join("aQb"), b"").expect("touch fixture");
    let script = format!(
        "cd '{}'; echo $'a*b'; g=$'a*b'; printf 'glob=%s\\n' $g",
        dir.display()
    );
    let out = rubash(&script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out, b"a*b\nglob=aQb\n");
}

#[test]
fn for_list_and_redirect_boundaries() {
    // array27.sub class: `for k in $'\t'` binds the bare tab; a redirect
    // operand with a decoded blank opens one literal file.
    let dir = std::env::temp_dir().join("rubash379rd");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir fixture");
    let script = format!(
        "cd '{}'; for k in $'\\t'; do printf 'k=%d\\n' \"${{#k}}\"; done; \
         echo hi > $'out tab.txt'; printf 'file=%s\\n' \"$(ls)\"",
        dir.display()
    );
    let out = rubash(&script);
    let _ = std::fs::remove_dir_all(&dir);
    if out != b"k=1\nfile=out tab.txt\n" {
        panic!("script={script:?} out={out:?}");
    }
    assert_eq!(out, b"k=1\nfile=out tab.txt\n");
}

#[test]
fn conditional_and_herestring_boundaries() {
    assert_eq!(
        rubash("[[ $'a b' == $'a b' ]] && echo dbr-ok || echo dbr-bad"),
        b"dbr-ok\n"
    );
    assert_eq!(rubash("cat <<< $'a b'"), b"a b\n");
}

#[test]
fn control_byte_data_still_stores_byte_exact() {
    // The guard codepoint lives OUTSIDE the raw-byte pair payload range
    // and the registry zone: decoded control bytes (0x11 here) keep
    // storing and printing exactly. (Reading a C0 cell back through a
    // command substitution is a separate pre-existing gap — rubash passes
    // the pair-encoded form there on HEAD too — so this asserts the
    // direct round-trip.)
    assert_eq!(
        rubash("v=$'\\021'; printf 'len=%d\\n' \"${#v}\"; printf '%s' \"$v\"; echo x"),
        "len=1\n\x11x\n".as_bytes()
    );
}

#[test]
fn standalone_ansic_controls_unchanged() {
    assert_eq!(rubash("echo $'a\\nb'"), b"a\nb\n");
    assert_eq!(rubash("echo M$'\\n'N"), b"M\nN\n");
    assert_eq!(rubash("echo pre$'\\t'mid$'\\t'post"), b"pre\tmid\tpost\n");
}
