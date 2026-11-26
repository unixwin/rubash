//! Issue rubash#354 (carrier family, cf. #296/#187/#179/#214): the
//! argument-position fragment splice (`expand_simple_substitution_fragments`
//! -> `split_expanded_fragments`) owner-tagged SOURCE-TEXT quote carriers as
//! payload data, so the argv materializer's carrier decode
//! (`materialize_expanded_command_word` -> `decode_word_position_carriers`)
//! no longer saw them and the final output boundary decoded the marker pair
//! to the raw carrier byte:
//!
//!   echo "\"$(echo x)\""   printed 0x18 0x78 0x18  (GNU: 22 78 22 = `"x"`)
//!
//! GNU contract (vendored third_party/bash): a character quoted in the
//! source word (parse.y:5694-5706 got_escaped_character) rides through
//! expansion CTLESC-protected (subst.c:11639-11673 SCOPY_CHAR_I) and the
//! protection is stripped only at argv materialization
//! (subst.c:4807 dequote_string) — it never reaches output as a control
//! byte. The fix records every carrier-family byte of a source-text
//! fragment as a live marker position in `split_expanded_fragments`
//! (substitution_metadata.rs), while capture bytes stay owner-tagged
//! payload: a 0x18 that came OUT of a command substitution is data and
//! must reach the output boundary as that byte.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

use std::process::Command;

fn rubash(script: &str) -> (Vec<u8>, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue reproducer: nested escaped-backslash comsub inside dquotes.
#[test]
fn escaped_backslash_comsub_in_dquotes_has_no_carrier_bytes() {
    let (stdout, stderr, code) = rubash(r#"echo "\"$(echo "\\")\"""#);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"\"\\\"\n");
}

/// Minimal form: escaped dquote adjacent to a comsub inside dquotes.
#[test]
fn escaped_dquote_around_comsub_decodes_to_quote() {
    let (stdout, _, code) = rubash(r#"echo "\"$(echo x)\"""#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"\"x\"\n");
}

/// Carrier mid-word with literal affixes (both fragments carry DATA_DQUOTE).
#[test]
fn carriers_span_literal_prefix_and_suffix_fragments() {
    let (stdout, _, code) = rubash(r#"echo "a\"b$(echo c)d\"e""#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"a\"bcd\"e\n");
}

/// Single-quote data carrier (`'` inside dquotes) rides the same splice.
#[test]
fn data_squote_carrier_survives_fragment_splice() {
    let (stdout, _, code) = rubash(r#"echo "a'b$(echo c)e'f""#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"a'bce'f\n");
}

/// A `\\` DATA_BACKSLASH carrier from the source word is data, and the
/// comsub capture backslash is data too: GNU prints `"\` + `\` + `"`.
#[test]
fn source_backslash_carrier_and_capture_backslash_both_literal() {
    let (stdout, _, code) = rubash(r#"echo "\"$(printf '\\')\"""#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"\"\\\"\n");
}

/// Provenance split (the invariant this fix restores): a 0x18 byte that
/// came OUT of a command substitution is PAYLOAD — it must reach the
/// output boundary as the raw byte, never as a `"` (decode_word_position_
/// carriers) or as a visible marker pair.
#[test]
fn capture_carrier_byte_is_payload_not_quote_syntax() {
    let (stdout, _, code) = rubash(r#"printf '%s' "a$(printf '\030')b" | od -An -tx1"#);
    assert_eq!(code, Some(0));
    let text = String::from_utf8_lossy(&stdout).into_owned();
    assert_eq!(text.trim(), "61 18 62");
}

/// Mixed provenance in one word: source `\"` carrier -> `"` at argv, capture
/// 0x18 -> raw byte at output.
#[test]
fn mixed_source_carrier_and_capture_payload() {
    let (stdout, _, code) = rubash(r#"printf '%s' "\"$(printf '\030')\"" | od -An -tx1"#);
    assert_eq!(code, Some(0));
    let text = String::from_utf8_lossy(&stdout).into_owned();
    assert_eq!(text.trim(), "22 18 22");
}

/// Unquoted (split-policy) splice keeps the carrier decode: GNU field-splits
/// only the capture; the quoted affixes stay attached.
#[test]
fn split_policy_keeps_source_carriers_live() {
    let (stdout, _, code) = rubash(r#"printf '[%s]\n' "a\"$(echo 'b c')\"d""#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"[a\"b c\"d]\n");
}

/// The assignment position was already correct (#296 family); pin it so the
/// fragment fix cannot regress the assignment path.
#[test]
fn assignment_position_quote_carriers_still_correct() {
    let (stdout, _, code) = rubash(r#"v="\"$(echo x)\""; printf '%s' "$v" | od -An -tx1"#);
    assert_eq!(code, Some(0));
    let text = String::from_utf8_lossy(&stdout).into_owned();
    assert_eq!(text.trim(), "22 78 22");
}
