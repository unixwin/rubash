//! Issue rubash#1927: a registry-zone glyph TYPED literally into a patsub
//! replacement body (raw UTF-8 in the source, no `$'...'` spelling) leaked
//! a stray U+E400 in front of the glyph: `${v/b/<U+E312>x}` printed
//! U+E400 U+E312 x where GNU prints U+E312 x (residual of the #843 fix).
//!
//! Mechanism: script entry escaping (bytes_to_script_text /
//! command_string_entry_text) already E400-escapes every typed zone glyph,
//! so the replacement text the mark pass walks carries the glyph as a
//! LITERAL_CHAR_ESCAPE pair. The mark pass's per-char literal arms escaped
//! each half a second time; the output boundary decodes exactly once, so
//! one introducer survived. The mark pass now copies an existing pair
//! verbatim as a unit (push_existing_literal_pair).
//!
//! GNU semantics: the replacement side is expanded as its own word
//! (subst.c:3950 expand_string_for_patsub) and its characters are plain
//! data — a typed glyph rides verbatim. Expected outputs below are the
//! GNU 5.3.20 bytes under a UTF-8 locale (the #843 locale gate); the
//! harness pins LC_ALL=C.utf8 for both the typed-glyph and the span
//! shapes so the assertions are locale-deterministic.

use std::process::Command;

fn rubash_stdout(script: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.utf8")
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "script failed: {script}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// The ticket reproduction: a literal U+E312 typed into the replacement
/// body emits exactly the glyph — no stray U+E400 introducer.
#[test]
fn literal_typed_glyph_unquoted() {
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/\u{E312}x}\"");
    assert_eq!(out, format!("a\u{E312}xc").into_bytes());
}

/// The same typed glyph inside a single-quoted replacement span.
#[test]
fn literal_typed_glyph_single_quoted() {
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/'\u{E312}x'}\"");
    assert_eq!(out, format!("a\u{E312}xc").into_bytes());
}

/// The same typed glyph inside a double-quoted replacement span.
#[test]
fn literal_typed_glyph_double_quoted() {
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/\"\u{E312}x\"}\"");
    assert_eq!(out, format!("a\u{E312}xc").into_bytes());
}

/// A backslash-escaped typed glyph: the backslash is consumed as quoting
/// and the glyph rides alone (GNU byte-verified).
#[test]
fn literal_typed_glyph_backslash_escaped() {
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/\\\u{E312}x}\"");
    assert_eq!(out, format!("a\u{E312}xc").into_bytes());
}

/// A typed glyph inside an ANSI-C span body (the literal form of the
/// `$'\uE312x'` shape that #843 already pinned).
#[test]
fn literal_typed_glyph_inside_ansi_c_span() {
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/$'\u{E312}x'}\"");
    assert_eq!(out, format!("a\u{E312}xc").into_bytes());
}

/// Zone glyphs typed literally, one per shape: the escape introducer
/// itself, the ANSI_C_IFS_GUARD codepoint, and the two PATSUB region
/// markers must each round-trip as exactly one glyph.
#[test]
fn literal_typed_zone_glyphs_roundtrip() {
    for cp in ['\u{E310}', '\u{E313}', '\u{E400}', '\u{E401}'] {
        let script = format!("v=abc\nprintf '%s' \"${{v/b/{cp}x}}\"");
        let out = rubash_stdout(&script);
        assert_eq!(
            out,
            format!("a{cp}xc").into_bytes(),
            "typed U+{:04X} must ride as exactly one glyph",
            cp as u32
        );
    }
}

/// Two adjacent typed glyphs (U+E400 then U+E312): each gets exactly its
/// own escape — the pair walk must not merge or double either.
#[test]
fn literal_typed_adjacent_glyphs_roundtrip() {
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/\u{E400}\u{E312}x}\"");
    assert_eq!(out, format!("a\u{E400}\u{E312}xc").into_bytes());
}

/// The raw-byte introducer U+E000 typed literally keeps its pre-#1927
/// behavior (it owns the raw-byte pair protocol and is excluded from the
/// literal-pair walk).
#[test]
fn literal_typed_e000_unchanged() {
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/\u{E000}x}\"");
    assert_eq!(out, format!("a\u{E000}xc").into_bytes());
}
