//! Issue rubash#843: user PUA codepoints in a patsub replacement text were
//! not E400-escaped, so they collided with the pipeline's own PATSUB
//! markers — `v=abc; r="${v/b/$'\uE312x'}"` produced `a&xc` (E312 was read
//! as PATSUB_QUOTED_AMP and rendered through the `\&` path), while
//! U+E310/U+E311 were silently consumed as quoted-value region markers and
//! dropped.
//!
//! GNU semantics (vendored third_party/bash): the replacement side of a
//! pattern substitution is expanded as its own word — subst.c:3950
//! expand_string_for_patsub, called at parameter_brace_patsub:9433 — and its
//! characters are plain data. `&`/`\&` reference forms and the `//` global
//! form keep working around the glyph; strcreplace (stringlib.c:180, flags
//! 2 via pat_subst:9252) only treats a literal `&`/`\&`/`\\` specially, so
//! a U+E3xx glyph rides the replacement verbatim.
//!
//! Fix: the replacement scan pass E400-escapes every literal registry-zone
//! char it copies (markers::push_literal_char, the #735 mechanism —
//! push_replacement_literal), the ANSI-C `$'...'` span escapes typed glyphs
//! at body collection, and finish_patsub_replacement copies E400/E000
//! escape pairs verbatim before resolving the markers; the Output boundary
//! restores the glyph exactly once. markers.rs registers the four PATSUB
//! codepoints as user_reachable with the in-flight escape contract.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (script-file probes oracle843.sh / oracle843b.sh, 2026-10-19): every
//! assertion below is byte-identical to the GNU oracle.

use std::process::Command;

fn rubash_stdout(script: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "script failed: {script}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// The ticket reproduction: E312 in an ANSI-C replacement is data, not
/// PATSUB_QUOTED_AMP — `a` + U+E312 + `x` + `c`, never `a&xc`.
#[test]
fn ansi_c_e312_replacement_is_literal_data() {
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/$'\\uE312x'}\"");
    assert_eq!(
        out,
        format!("a\u{E312}xc").into_bytes(),
        "E312 replacement must ride as the literal glyph"
    );
}

/// E310/E311 were consumed as quoted-value region markers and dropped.
#[test]
fn ansi_c_e310_e311_e313_replacements_are_literal_data() {
    for (cp, spelling) in [
        ('\u{E310}', "\\uE310"),
        ('\u{E311}', "\\uE311"),
        ('\u{E313}', "\\uE313"),
    ] {
        let script = format!("v=abc\nprintf '%s' \"${{v/b/$'{spelling}x'}}\"");
        let out = rubash_stdout(&script);
        assert_eq!(
            out,
            format!("a{cp}xc").into_bytes(),
            "U+{:04X} replacement must ride as the literal glyph",
            cp as u32
        );
    }
}

/// The `&` match-reference form around a PUA glyph: `&` still copies the
/// match AND the glyph survives (`abc/b/&$'\uE312'` -> ab + E312 + c).
#[test]
fn ampersand_reference_with_pua_glyph() {
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/&$'\\uE312'}\"");
    assert_eq!(out, format!("ab\u{E312}c").into_bytes());
}

/// The `\&` literal-ampersand form next to a PUA glyph.
#[test]
fn escaped_ampersand_with_pua_glyph() {
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/\\&$'\\uE312'}\"");
    assert_eq!(out, format!("a&\u{E312}c").into_bytes());
}

/// Global `//` substitution: every match gets the glyph-bearing
/// replacement, nothing is dropped between matches.
#[test]
fn global_substitution_with_pua_glyph() {
    let out = rubash_stdout("w=aAbAbA\nprintf '%s' \"${w//b/$'\\uE312x'}\"");
    assert_eq!(out, format!("aA\u{E312}xA\u{E312}xA").into_bytes());
    let out = rubash_stdout("w=aAbAbA\nprintf '%s' \"${w//A/&$'\\uE310'}\"");
    assert_eq!(out, format!("aA\u{E310}bA\u{E310}bA\u{E310}").into_bytes());
}

/// The replacement arrives from a stored variable: storage text carries
/// the E400 pair, the expansion result must not be read as a marker.
#[test]
fn variable_expanded_replacement_with_pua_glyph() {
    let out = rubash_stdout("rep=$'\\uE312x'\nv=abc\nprintf '%s' \"${v/b/$rep}\"");
    assert_eq!(out, format!("a\u{E312}xc").into_bytes());
    let out = rubash_stdout("rep=$'\\uE310x'\nv=abc\nprintf '%s' \"${v/b/$rep}\"");
    assert_eq!(out, format!("a\u{E310}xc").into_bytes());
}

/// A raw-byte marker pair whose payload lands in E310..=E313 (bytes
/// 0x10-0x13) must decode to the byte, never be eaten as a marker.
#[test]
fn raw_byte_payload_rides_verbatim() {
    for byte in ['\u{0010}', '\u{0011}', '\u{0012}', '\u{0013}'] {
        let n = byte as u32;
        let script = format!("v=abc\nprintf '%s' \"${{v/b/$'\\x{n:02x}x'}}\"");
        let out = rubash_stdout(&script);
        assert_eq!(
            out,
            format!("a{byte}xc").into_bytes(),
            "raw byte 0x{n:02x} in the replacement must survive"
        );
    }
}

/// `declare -p` renders the glyph (storage holds the E400 pair; the
/// display boundary decodes it once).
#[test]
fn declare_p_shows_the_glyph() {
    let out = rubash_stdout("v=abc\nr=\"${v/b/$'\\uE312x'}\"\ndeclare -p r");
    assert_eq!(out, format!("declare -- r=\"a\u{E312}xc\"\n").into_bytes());
}

/// A literal PUA glyph typed directly into the replacement body (raw
/// UTF-8 in the script, no `$'...'` quoting) round-trips too.
#[test]
fn literal_typed_glyph_in_replacement_body() {
    let script = "v=abc\nprintf '%s' \"${v/b/\u{E312}x}\"";
    let out = rubash_stdout(script);
    assert_eq!(out, format!("a\u{E312}xc").into_bytes());
    let script = "v=abc\nprintf '%s' \"${v/b/'\u{E312}x'}\"";
    let out = rubash_stdout(script);
    assert_eq!(out, format!("a\u{E312}xc").into_bytes());
}

/// Zone edges: the E400 introducer itself and the ANSI_C_IFS_GUARD
/// codepoint are user data here, and a quoted `&` after a literal E400
/// glyph stays a literal ampersand (GNU: `a` + E400 + `\` + `&` + `c`,
/// the `\&` unrecognized-escape keeps its backslash).
#[test]
fn zone_edge_glyphs_roundtrip() {
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/$'\\uE400x'}\"");
    assert_eq!(out, format!("a\u{E400}xc").into_bytes());
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/$'\\uE401x'}\"");
    assert_eq!(out, format!("a\u{E401}xc").into_bytes());
    let out = rubash_stdout("v=abc\nprintf '%s' \"${v/b/$'\\uE400\\&'}\"");
    assert_eq!(out, format!("a\u{E400}\\&c").into_bytes());
}

/// shopt patsub_replacement off: quote removal only — the glyph still
/// rides verbatim and a plain `&` stays literal (no MATCH_EXPREP,
/// subst.c:9430-9431).
#[test]
fn shopt_off_keeps_glyph_and_literal_ampersand() {
    let script = "shopt -u patsub_replacement\nv=abc\nprintf '%s' \"${v/b/$'\\uE312x'}\"\nprintf '\\n'\nprintf '%s' \"${v/b/X&Y}\"\n";
    let out = rubash_stdout(script);
    assert_eq!(out, format!("a\u{E312}xc\naX&Yc").into_bytes());
}

/// No transport residue: the patsub output must not carry a stray E400
/// introducer (a split pair would eat the following character at the
/// decode boundary).
#[test]
fn output_carries_no_stray_escape_introducer() {
    let script = "v=abc\nr=\"${v/b/$'\\uE312x'}\"\nprintf '%s' \"$r\" | od -An -tx1";
    let raw = rubash_stdout(script);
    let out = String::from_utf8_lossy(&raw);
    assert_eq!(out, " 61 ee 8c 92 78 63\n");
}
