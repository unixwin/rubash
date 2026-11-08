//! Decoder-coupled registry tests (rubash#728 §3).
//!
//! These two tests live here — not in `src/executor/markers.rs`'s own
//! `#[cfg(test)]` module — because they call the real decoders through
//! crate paths (`substitution_metadata::decode_raw_byte_markers`,
//! `locale::decode_to_visible_text`). The registry file is also compiled
//! into the `tests/marker_registry_roundtrips.rs` crate via `#[path]`,
//! where no `crate::executor` / `crate::locale` modules exist; its own
//! test module must therefore stay self-contained (registry table +
//! std only). Moving these two tests out is what makes that include
//! compile. They still run in the lib suite, same assertions, same
//! pass — only their home changed.

use crate::executor::markers::{
    encode_raw_byte_marker, is_marker_zone_char, CASE_PATTERN_EXPECT, CASE_PATTERN_MAY_OPEN,
    CASE_PATTERN_PAREN, COMPOUND_ASSIGNMENT_MARKER, COMSUB_PAYLOAD_SENTINEL_STR,
    FAILED_SUBSCRIPT_SENTINEL, GROUP_REDIRECT_INJECTED_MARK, MARKERS, QUOTED_HEREDOC_MARKER,
    RAW_BYTE_MARKER_FIRST,
};
use crate::executor::substitution_metadata::decode_raw_byte_markers;
use crate::locale::decode_to_visible_text;

/// `user_reachable` is machine-read (746 case 728.1): a user-reachable
/// marker either is a single C0/ASCII byte — covered by the raw-byte
/// pair re-encode, asserted for every possible byte below — or a PUA
/// codepoint inside the E400 literal-char-escape zone, which is how
/// user data is disambiguated at entry (#735). A `user_reachable: true`
/// row outside both shapes has no re-encode story and must not exist.
#[test]
fn user_reachable_rows_have_an_entry_reencode_story() {
    for byte in 0..=255u8 {
        let encoded = encode_raw_byte_marker(byte);
        let decoded = decode_raw_byte_markers(encoded.as_bytes());
        assert_eq!(decoded, vec![byte], "raw-byte pair lost byte {byte:#04x}");
    }
    for m in MARKERS {
        if !m.user_reachable {
            continue;
        }
        assert!(
            m.code <= 0xff || is_marker_zone_char(char::from_u32(m.code).unwrap()),
            "{} claims user_reachable at U+{:04X} but has no entry re-encode",
            m.name,
            m.code
        );
    }
}

/// `boundaries` is machine-read (746 case 728.1): a row that claims an
/// output/storage/reparse decode must be named by the decode machinery
/// that owns it — decode_to_visible_text for Output rows (the
/// sanctioned transport→visible decoder, governance 3.2), or an
/// explicitly listed owner for markers consumed by their own boundary
/// helper (echo's $'...' fast path, substitution_metadata, ...). The
/// owner lists are pinned: adding a row with a boundary requires
/// naming its decoder here, which is exactly the audit trail #728
/// asked for.
#[test]
fn boundary_claims_name_a_decoder() {
    // Markers decode_to_visible_text consumes (verified live below).
    const VISIBLE_DECODER_OUTPUTS: [&str; 22] = [
        "RAW_BYTE_MARKER_ESCAPE",
        "CTLESC",
        "PARAM_NAME_END_MARKER",
        "DATA_BACKSLASH",
        "PROTECTED_ESCAPED_SQUOTE",
        "DATA_SQUOTE",
        "DATA_DQUOTE",
        "PATTERN_LITERAL_BACKSLASH",
        "DATA_BACKTICK",
        "QUOTED_WORD_PREFIX",
        "IFS_GLUE",
        "STORAGE_WORD_PREFIX",
        "DATA_DOLLAR",
        "DEFERRED_COMPOUND_BODY",
        "ARRAYREF_FLAG",
        "PATSUB_QUOTED_VALUE_START",
        "PATSUB_QUOTED_VALUE_END",
        "PATSUB_QUOTED_AMP",
        "PATSUB_QUOTED_BACKSLASH",
        "QUOTED_NULL_MARKER",
        "FAILED_SUBSCRIPT_SENTINEL",
        "LITERAL_CHAR_ESCAPE",
    ];
    // Markers whose named boundary is consumed by their own helper
    // (documented at the row): owner decode fn, not the visible decoder.
    const OWN_BOUNDARY_DECODERS: [&str; 41] = [
        "PROTECTED_BACKSLASH",         // builtins/echo.rs $'...' fast path
        "PROTECTED_LITERAL_BACKSLASH", // embedded_parameters walker tail
        "PROTECTED_LITERAL_DOLLAR",    // embedded_parameters walker tail
        "ANSI_C_QUOTE_MARKER",         // decode_word_position_carriers / eval_source_for_reparse
        "ANSI_C_DQUOTE_MARKER",        // (also in the visible decoder list)
        "ANSI_C_IFS_GUARD",            // markers::strip_ansi_c_ifs_guard
        // The backslash/IFS guard family (rubash#2885 E8): each
        // guard is a protect-prefix consumed with its data char by
        // its owning walker, and by the visible decoder's guard
        // arm (locale.rs) as the output-side backstop.
        "PARAM_WORD_BACKSLASH_GUARD", // parameter_words walker + locale.rs guard arm
        "ESCAPED_IFS_GUARD",          // command_prepare field-split restore + locale.rs guard arm
        "PROMPT_ESCAPE_GUARD",        // assignment_expansion preserve/restore + locale.rs guard arm
        "CASE_PATTERN_BACKSLASH_GUARD", // compound_exec case-pattern walker + locale.rs guard arm
        "SUBSCRIPT_CARRIER",          // substitution_metadata subscript decode
        "PARSE_ERROR_FIELD_SEP",      // parse_error payload split
        "HEREDOC_WARNED_BODY_PREFIX", // execution_misc heredoc body strip
        "QUOTED_WORD_VALUE_PREFIX",   // expand/tilde QUOTED_ASSIGNMENT_VALUE
        "HASH_ENV_FIELD_SEP",         // builtins/hash env serialization
        "DIR_STACK_FIELD_SEP",        // builtins/pushd env serialization
        "PROMPT_IGNORE_START",        // readline prompt-ignore layer
        "PROMPT_IGNORE_END",          // readline prompt-ignore layer
        "PREEXPANDED_STDIN_BODY",     // execution_misc body consumer + parser raw-byte pair
        "ARRAY_FIELD_SPLIT_MARKER",   // declare/storage/array element split
        "QUOTED_HEREDOC_MARKER", // execution_misc.rs body strip (QUOTED_HEREDOC_MARKER replace)
        "COMPOUND_ASSIGNMENT_MARKER", // types.rs compound-operand strip
        "COMSUB_PAYLOAD_SENTINEL", // execution_misc decode_command_substitution_payload
        "CASE_PATTERN_EXPECT",   // parser/support.rs boundary-stack sentinel
        "CASE_PATTERN_MAY_OPEN", // parser/support.rs boundary-stack sentinel
        "CASE_PATTERN_PAREN",    // parser/support.rs boundary-stack sentinel
        // The ASSIGN_* family restores through the assignment_expansion
        // storage-restore chain (rubash#295/#218 restore tails; the
        // #811 fragment-boundary strip).
        "ASSIGN_DATA_SQUOTE",
        "ASSIGN_DATA_DQUOTE",
        "ASSIGN_DATA_BACKTICK",
        "ASSIGN_ESCAPED_DQUOTE",
        "ASSIGN_ESCAPED_SQUOTE",
        "ASSIGN_ESCAPED_BACKSLASH",
        "ASSIGN_HOISTED_SQUOTE",
        "ASSIGN_HOISTED_BACKSLASH",
        "COMPOUND_EXPANSION_WS_TAG",
        "ASSIGN_SQ_DOLLAR",
        "ASSIGN_SQ_BACKTICK",
        "ASSIGN_SQ_BACKSLASH",
        "ASSIGN_EXPANSION_BACKSLASH",
        "ASSIGN_EXPANSION_DQUOTE",
        "ASSIGN_EXPANSION_SQUOTE",
    ];
    // Multi-char string rows register their FIRST codepoint; keep the
    // table honest against the consts.
    for (name, first) in [
        ("FAILED_SUBSCRIPT_SENTINEL", FAILED_SUBSCRIPT_SENTINEL),
        ("QUOTED_HEREDOC_MARKER", QUOTED_HEREDOC_MARKER),
        ("COMSUB_PAYLOAD_SENTINEL", COMSUB_PAYLOAD_SENTINEL_STR),
        ("COMPOUND_ASSIGNMENT_MARKER", COMPOUND_ASSIGNMENT_MARKER),
        ("GROUP_REDIRECT_INJECTED_MARK", GROUP_REDIRECT_INJECTED_MARK),
        ("CASE_PATTERN_EXPECT", CASE_PATTERN_EXPECT),
        ("CASE_PATTERN_MAY_OPEN", CASE_PATTERN_MAY_OPEN),
        ("CASE_PATTERN_PAREN", CASE_PATTERN_PAREN),
    ] {
        let row = MARKERS
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("{name} missing from MARKERS"));
        assert_eq!(
            row.code,
            first.chars().next().unwrap() as u32,
            "{name} registered codepoint does not match its const"
        );
    }
    for m in MARKERS {
        if m.boundaries.is_empty() {
            continue;
        }
        let named =
            VISIBLE_DECODER_OUTPUTS.contains(&m.name) || OWN_BOUNDARY_DECODERS.contains(&m.name);
        assert!(
            named,
            "{} claims boundaries {:?} but no decoder owns it (rubash#728)",
            m.name, m.boundaries
        );
        // Live check for the visible-decoder claims: a bare marker char
        // must not survive decode_to_visible_text unchanged. The E000
        // introducer decodes as a PAIR (E000+E001 = byte 0x01); a bare
        // introducer is a documented recovery passthrough.
        if VISIBLE_DECODER_OUTPUTS.contains(&m.name) {
            if m.name == "RAW_BYTE_MARKER_ESCAPE" {
                let pair = format!(
                    "{}{}",
                    char::from_u32(m.code).unwrap(),
                    char::from_u32(RAW_BYTE_MARKER_FIRST).unwrap()
                );
                let decoded = decode_to_visible_text(&pair);
                // E000 + E001 is byte 0x00 (payload = FIRST + byte).
                assert_eq!(
                    decoded, "\u{0}",
                    "RAW_BYTE_MARKER_ESCAPE pair must decode to its byte"
                );
            } else if m.name == "LITERAL_CHAR_ESCAPE" {
                // E400 + c restores c (bare E400 is a documented literal
                // passthrough).
                let pair = format!("{}x", char::from_u32(m.code).unwrap());
                assert_eq!(
                    decode_to_visible_text(&pair),
                    "x",
                    "LITERAL_CHAR_ESCAPE pair must restore the escaped char"
                );
            } else if let Some(ch) = char::from_u32(m.code) {
                let text = ch.to_string();
                let decoded = decode_to_visible_text(&text);
                assert_ne!(
                    decoded, text,
                    "{} claims Output/Storage decode but decode_to_visible_text passes it through",
                    m.name
                );
            }
        }
    }
}
