//! Issue rubash#734: associative subscripts written through `$'...'` (ANSI-C
//! quoting) did not match the same key looked up through another entry point.
//!
//! GNU bash 5.3.0 baseline (WSL, UTF-8 locale, byte-verified probes):
//!
//! 1. `A[$'x\ty']=1` then `${A[$k]}` with `k=$'x\ty'` printed empty — the
//!    write stored the key with the internal ANSI_C_IFS_GUARD marker
//!    (U+E401, emitted before the decoded blank) while a variable/dq lookup
//!    produced the clean bytes, so no spelling pair agreed except a
//!    `$'...'`-to-`$'...'` round trip.
//! 2. `declare -p` leaked the marker into the rendered key:
//!    `([$'x` + `ee 90 81` + `\ty']--)` instead of GNU's `([$'x\ty'])`.
//! 3. `A[$'a\\b']=1` stored `ab` (61 62) instead of `a\b` (61 5c 62): the
//!    assignment path re-decoded the raw `$'...'` text and the follow-up
//!    escape pass ate the decoded backslash. GNU's LEXER decodes the
//!    assignment word at parse time with CTLESC protection, so the decoded
//!    backslash is quoted data and survives (arrayfunc.c:392-398
//!    assign_array_element_internal -> subst.c:11063
//!    expand_subscript_string). The `${A[...]}` reader re-decodes the raw
//!    text unquoted at runtime, so GNU's own read of `${A[$'a\\b']}` keys on
//!    `ab` and MISSES the `a\b` cell — an asymmetry pinned here as parity.
//!
//! Fix: `expand_subscript_string` strips the bare (E400-unescaped) guard
//! from the resolved key, so every spelling stores and looks up the same
//! clean cell; the assignment entries use
//! `expand_assignment_subscript_string`, which protects decoded `$'...'`
//! backslashes the way GNU's parse-time decode does.
//!
//! Expected outputs are byte-verified against GNU bash 5.3.0 (script-file
//! probes, 2026-10-13; the full 25-row differential is on the PR).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
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

/// Item 1 headline repro: a key written through `$'...'` must be readable
/// through a variable holding the same bytes (GNU: `[1]`).
#[test]
fn write_ansi_c_read_variable() {
    let (stdout, stderr, _) =
        rubash("k=$'x\\ty'; declare -A A; A[$'x\\ty']=1; printf '[%s]\\n' \"${A[$k]}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "[1]\n");
}

/// Reverse direction: write through a variable, read through `$'...'`.
#[test]
fn write_variable_read_ansi_c() {
    let (stdout, stderr, _) =
        rubash("k=$'x\\ty'; declare -A A; A[$k]=1; printf '[%s]\\n' \"${A[$'x\\ty']}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "[1]\n");
}

/// Cross-spelling agreement with a double-quoted literal tab, both ways.
#[test]
fn cross_spelling_dq_literal_tab() {
    let (stdout, stderr, _) = rubash(
        "declare -A A; A[$'x\\ty']=1; printf '[%s]\\n' \"${A[\"x\ty\"]}\";
         declare -A B; B[\"x\ty\"]=1; printf '[%s]\\n' \"${B[$'x\\ty']}\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "[1]\n[1]\n");
}

/// The stored key is the clean decoded bytes `x TAB y` (GNU 78 09 79), not a
/// marker-laden variant.
#[test]
fn ansi_c_tab_key_storage_bytes() {
    let (stdout, stderr, _) = rubash(
        "declare -A A; A[$'x\\ty']=1; for k in \"${!A[@]}\"; do printf '%s' \"$k\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'; done",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "780979\n");
}

/// Item 3: `$'a\\b'` stores the decoded backslash (`a\b`, 61 5c 62), like
/// GNU's parse-time decode — not the eaten `ab`.
#[test]
fn ansi_c_backslash_key_storage_bytes() {
    let (stdout, stderr, _) = rubash(
        "declare -A A; A[$'a\\\\b']=1; for k in \"${!A[@]}\"; do printf '%s' \"$k\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'; done",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "615c62\n");
}

/// GNU's read-side asymmetry, pinned as parity: the `${...}` reader decodes
/// `$'a\\b'` unquoted at runtime, so its backslash drops and the lookup keys
/// on `ab` — missing the `a\b` cell — while the literal spellings hit it.
#[test]
fn ansi_c_backslash_read_asymmetry_matches_gnu() {
    let (stdout, stderr, _) = rubash(
        "declare -A A; A[$'a\\\\b']=1
         printf '[%s]\\n' \"${A[$'a\\\\b']}\"   # ansi self-read: GNU miss
         printf '[%s]\\n' \"${A[a\\\\b]}\"     # bare escape: GNU hit
         printf '[%s]\\n' \"${A['a\\b']}\"    # single-quoted: GNU hit",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "[]\n[1]\n[1]\n");
}

/// `$'a\\b'` and `a\\b` writes collapse onto ONE cell (GNU: `["a\\b"]`).
#[test]
fn backslash_spellings_collapse_to_one_cell() {
    let (stdout, stderr, _) =
        rubash("declare -A A; A[$'a\\\\b']=first; A[a\\\\b]=second; declare -p A");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A A=([\"a\\\\b\"]=\"second\" )\n");
}

/// `$'x\ty'` and a dq literal-tab write collapse onto ONE cell rendered in
/// GNU's `$'...'` form.
#[test]
fn tab_spellings_collapse_to_one_cell() {
    let (stdout, stderr, _) =
        rubash("declare -A A; A[$'x\\ty']=first; A[\"x\ty\"]=second; declare -p A");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A A=([$'x\\ty']=\"second\" )\n");
}

/// Item 2: `declare -p` renders the key in GNU's ANSI-C form
/// (`[$'x\ty']`) with NO marker bytes (U+E401 would appear as ee 90 81).
#[test]
fn declarep_renders_clean_ansi_c_key() {
    let (stdout, stderr, _) =
        rubash("declare -A A; A[$'x\\ty']=1; declare -p A | od -A n -t x1 | tr -d ' \\n'");
    assert_clean(&stderr);
    assert_eq!(
        stdout,
        "6465636c617265202d4120413d285b2427785c7479275d3d22312220290a"
    );
    assert!(!stdout.contains("ee9081"), "marker leaked into declare -p");
}

/// Membership agrees across spellings: `${A[$k]+x}`, `${A[$'x\ty']+x}` and
/// `[[ -v A[$k] ]]` all report the cell set.
#[test]
fn membership_agrees_across_spellings() {
    let (stdout, stderr, _) = rubash(
        "k=$'x\\ty'; declare -A A; A[$'x\\ty']=1
         [[ -n \"${A[$k]+x}\" ]] && echo set || echo unset
         [[ -n \"${A[$'x\\ty']+x}\" ]] && echo set || echo unset
         [[ -v A[$k] ]] && echo set || echo unset",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "set\nset\nset\n");
}

/// `unset "A[$k]"` removes the `$'...'`-written cell (GNU: `A=()`).
#[test]
fn unset_via_variable_removes_ansi_c_cell() {
    let (stdout, stderr, _) =
        rubash("k=$'x\\ty'; declare -A A; A[$'x\\ty']=1; unset \"A[$k]\"; declare -p A");
    assert_clean(&stderr);
    assert_eq!(stdout, "declare -A A=()\n");
}

/// The compound kvlist entry (`A=([...]=v)`) already normalized; it must
/// keep agreeing with the direct entry after the fix (GNU: `[cv]`).
#[test]
fn compound_kvlist_still_agrees_with_direct_entry() {
    let (stdout, stderr, _) =
        rubash("k=$'x\\ty'; declare -A A; A=([$'x\\ty']=cv); printf '[%s]\\n' \"${A[$k]}\"");
    assert_clean(&stderr);
    assert_eq!(stdout, "[cv]\n");
}

/// The compound kvlist entry stores `$'a\\b'` with the backslash kept
/// (GNU: 61 5c 62).
#[test]
fn compound_kvlist_backslash_key_bytes() {
    let (stdout, stderr, _) = rubash(
        "declare -A A; A=([$'a\\\\b']=w); for k in \"${!A[@]}\"; do printf '%s' \"$k\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'; done",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "615c62\n");
}

/// rubash#735 interplay: a USER-typed `$'\uE401'` rides the E400 pair, so
/// the bare-guard strip preserves it — the key bytes stay `ee 90 81` and the
/// variable-spelling read hits (GNU: `ee9081` / `[g]`).
#[test]
fn user_pua_e401_key_survives_guard_strip() {
    let (stdout, stderr, _) = rubash(
        "e=$'\\uE401'; declare -A A; A[$'\\uE401']=g
         for k in \"${!A[@]}\"; do printf '%s' \"$k\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'; done
         printf '[%s]\\n' \"${A[$e]}\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "ee9081\n[g]\n");
}

/// An unrecognized ANSI-C escape keeps its backslash in the stored key
/// (`$'p\q'` -> 70 5c 71, GNU-verified); the runtime-decoded read drops it,
/// so the self-read misses exactly like GNU.
#[test]
fn unrecognized_ansi_c_escape_keeps_backslash() {
    let (stdout, stderr, _) = rubash(
        "declare -A A; A[$'p\\q']=v
         for k in \"${!A[@]}\"; do printf '%s' \"$k\" | od -A n -t x1 | tr -d ' \\n'; printf '\\n'; done
         printf '[%s]\\n' \"${A[$'p\\q']}\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "705c71\n[]\n");
}
