//! rubash#3016 — U+E316 (PROMPT_ESCAPE_GUARD) typed-data folds to a
//! backslash in assignment concatenation.
//!
//! Root cause: `src/executor/assignment_expansion.rs::preserve_prompt_escapes`
//! ends with a blind `.replace(PROMPT_ESCAPE_GUARD, "\\")`. Typed
//! `$'\uE316'` arrives at the assignment expander as the E400 literal-char
//! escape pair (`E400 + E316`, `lexer/ansi.rs` entry via
//! `markers::push_literal_char`, rubash#735 rule), so the blind replace
//! reads the ESCAPED member of a user pair as live protocol, consumes the
//! E316, and leaves a stray E400 — which the output decoder then folds
//! to the bare backslash (`locale.rs` LITERAL_CHAR_ESCAPE arm emits the
//! char alone). Same family as rubash#840 divergence 3 (blind
//! `.replace(E30D, "\\")` chains superseded by
//! `markers::restore_bare_sentinels`) and rubash#735 (naive
//! `.replace(E401, "")`).
//!
//! Only the quoted-concat assignment path folds: the plain/braced-only
//! assignments (`w=$v`, `w=${v}`) are unquoted and never reach the
//! restore, and direct use (`printf '%s' "a${v}b"`) never passes through
//! `preserve_prompt_escapes`. Neighbour guards E315/E317 survive the
//! same shape — their restores are already E400-aware.
//!
//! GNU anchors: subst.c:4309 `expand_string_internal` expands the value
//! and variables.c `bind_variable` stores it verbatim — no step maps a
//! U+E316 data char to a backslash. Oracle evidence: the byte tables
//! below, GNU bash 5.3.20 vs rubash master 446d7662 (`LC_ALL=C.UTF-8`;
//! under a C locale `$'\uE316'` stays six literal characters on both
//! sides, rubash#1837).
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions. `LC_ALL=C.UTF-8` is set on the child explicitly so the
//! pin is hermetic under any ambient locale.

use std::process::{Command, Stdio};

/// U+E316 in UTF-8.
const E316_UTF8: &[u8] = "\u{E316}".as_bytes();
/// U+E315 in UTF-8.
const E315_UTF8: &[u8] = "\u{E315}".as_bytes();
/// U+E317 in UTF-8.
const E317_UTF8: &[u8] = "\u{E317}".as_bytes();

fn run(script: &str) -> Vec<u8> {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let output = child.wait_with_output().expect("wait rubash");
    assert!(
        output.status.success(),
        "rubash -c {script:?} exited with {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn expect(script: &str, expected: &[u8], ctx: &str) {
    let out = run(script);
    assert_eq!(out, expected, "{ctx}: got {out:?}, want {expected:?}");
}

#[test]
fn e316_braced_assignment_concat_preserved() {
    let mut want = vec![b'a'];
    want.extend_from_slice(E316_UTF8);
    want.push(b'b');
    expect(
        "v=$'\\uE316'; w=\"a${v}b\"; printf '%s' \"$w\"",
        &want,
        "braced concat w=\"a${v}b\" must keep U+E316",
    );
}

#[test]
fn e316_unbraced_assignment_concat_preserved() {
    let mut want = vec![b'a'];
    want.extend_from_slice(E316_UTF8);
    want.extend_from_slice(b" b");
    expect(
        "v=$'\\uE316'; w=\"a$v b\"; printf '%s' \"$w\"",
        &want,
        "unbraced concat w=\"a$v b\" must keep U+E316",
    );
}

#[test]
fn e316_direct_use_control() {
    let mut want = vec![b'a'];
    want.extend_from_slice(E316_UTF8);
    want.push(b'b');
    expect(
        "v=$'\\uE316'; printf '%s' \"a${v}b\"",
        &want,
        "direct use must keep U+E316 (never regressed)",
    );
}

#[test]
fn e316_plain_and_braced_assignment_control() {
    expect(
        "v=$'\\uE316'; w=$v; printf '%s' \"$w\"",
        E316_UTF8,
        "plain w=$v must keep U+E316 (unquoted path, never regressed)",
    );
    expect(
        "v=$'\\uE316'; w=${v}; printf '%s' \"$w\"",
        E316_UTF8,
        "braced-only w=${v} must keep U+E316 (unquoted path, never regressed)",
    );
}

#[test]
fn e316_with_prompt_escape_preserved() {
    // Sharpest cell: user E316 adjacent to a genuine `\!` guard. Base
    // folds the E316 to a backslash AND keeps the escape's backslash,
    // yielding a doubled backslash (`a\\!b`).
    let mut want = vec![b'a'];
    want.extend_from_slice(E316_UTF8);
    want.extend_from_slice(b"\\!b");
    expect(
        "v=$'\\uE316'; w=\"a${v}\\!b\"; printf '%s' \"$w\"",
        &want,
        "E316 next to a genuine \\! must survive, escape intact",
    );
}

#[test]
fn prompt_escape_bang_hash_preserved() {
    // Guards the purpose of preserve_prompt_escapes itself: `\!` / `\#`
    // in a quoted assignment survive as backslash + char.
    expect(
        "w=\"a\\!b\"; printf '%s' \"$w\"",
        b"a\\!b",
        "quoted \\! must stay backslash + bang",
    );
    expect(
        "w=\"a\\#b\"; printf '%s' \"$w\"",
        b"a\\#b",
        "quoted \\# must stay backslash + hash",
    );
}

#[test]
fn e315_e317_neighbour_guards_preserved() {
    let mut want315 = vec![b'a'];
    want315.extend_from_slice(E315_UTF8);
    want315.push(b'b');
    expect(
        "v=$'\\uE315'; w=\"a${v}b\"; printf '%s' \"$w\"",
        &want315,
        "neighbour guard E315 must survive the same shape",
    );
    let mut want317 = vec![b'a'];
    want317.extend_from_slice(E317_UTF8);
    want317.push(b'b');
    expect(
        "v=$'\\uE317'; w=\"a${v}b\"; printf '%s' \"$w\"",
        &want317,
        "neighbour guard E317 must survive the same shape",
    );
}

#[test]
fn c0_byte_in_assignment_concat_preserved() {
    expect(
        "v=$'\\x11'; w=\"a${v}b\"; printf '%s' \"$w\"",
        b"a\x11b",
        "C0 data byte must survive assignment concat (control)",
    );
}
