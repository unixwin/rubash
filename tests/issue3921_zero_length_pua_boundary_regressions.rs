//! Issue unixwin/rubash#3921: residual of #3546 (fixed by PR #3920,
//! 47a24b28), which moved the three replace finders to match-unit
//! boundaries but left two boundary faces behind:
//!
//! 1. Zero-length-match advance: `replace_parameter_pattern_inner`
//!    copies ONE char past a zero-length match (`cursor = end +
//!    ch.len_utf8()`). When the match lands exactly at a raw-byte
//!    marker pair (U+E000 + payload) — or a literal-char escape pair
//!    (U+E400 + char) — that copies only the pair's first half and
//!    strands `cursor` inside the pair. The scan then finds a phantom
//!    zero-length match at the end position, so every value whose LAST
//!    unit is a pair gains one spurious trailing replacement
//!    (`${w//*(x)/Q}` on a lone 0x14 byte is `Q\x14Q`, GNU: `Q\x14`).
//!    The advance must land on a match-unit boundary, exactly like
//!    the #3920 finder filter.
//!
//! 2. Literal U+E000 user data: a genuine U+E000 character in user
//!    data must behave as ONE ordinary data character in the replace
//!    face (`?`, `[...]`, literal pattern, `#`/`%` removal) — the
//!    same unit the removal walk already treats it as — instead of
//!    being fused with a following payload-range char or split from
//!    its escape half. If the two spellings (lone E000 vs. the
//!    transport pair) cannot be told apart cleanly, the behaviour is
//!    at least pinned here and aligned with the removal side.
//!
//! GNU anchor: subst.c:9184 pat_subst() hands the pattern to
//! lib/glob/sm_loop.c:398 BRACKMATCH(); pat_subst:9286-9304 copies
//! one CHARACTER past a zero-length match, and in GNU a carrier byte
//! (one byte) and a U+E000 char (one char) are each one character.
//!
//! Every expected value below is GNU bash 5.3.20 output for the same
//! script under LC_ALL=C.UTF-8 (the `\u` decoding of `$'\ue000'`
//! needs a UTF-8 locale; the `\x` carrier cases are locale-free).
//! The `*(x)` / `?(x)` zero-length probes need `shopt -s extglob`.

use std::process::{Command, Stdio};

fn run_file(script: &str, tag: &str) -> (String, String, i32) {
    let path = std::env::temp_dir().join(format!("i3921_{tag}_{}.sh", std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("LC_ALL", "C.UTF-8")
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// Item 1: a zero-length `*(x)` match landing on a carrier pair must
/// advance past the WHOLE pair. GNU `Q` counts: one Q per data unit,
/// no phantom trailing Q when the last unit is a pair.
#[test]
fn zero_length_match_advance_does_not_split_carrier_pair() {
    // (tag, script, expected stdout)
    for (tag, script, expected) in [
        (
            "lone_carrier",
            "shopt -s extglob\nc=$'\\x14'\nw=\"$c\"\nprintf '%s' \"${w//*(x)/Q}\"\n",
            "Q\u{14}",
        ),
        (
            "carrier_at_end",
            "shopt -s extglob\nc=$'\\x14'\nw=\"a${c}\"\nprintf '%s' \"${w//*(x)/Q}\"\n",
            "QaQ\u{14}",
        ),
        (
            "carrier_mid",
            "shopt -s extglob\nc=$'\\x14'\nw=\"a${c}b\"\nprintf '%s' \"${w//*(x)/Q}\"\n",
            "QaQ\u{14}Qb",
        ),
        (
            "two_carriers",
            "shopt -s extglob\nc=$'\\x14'\nw=\"${c}${c}\"\nprintf '%s' \"${w//*(x)/Q}\"\n",
            "Q\u{14}Q\u{14}",
        ),
        (
            "carrier_at_both_ends",
            "shopt -s extglob\nc=$'\\x14'\nw=\"a${c}b${c}\"\nprintf '%s' \"${w//*(x)/Q}\"\n",
            "QaQ\u{14}QbQ\u{14}",
        ),
        (
            "carrier_pair_then_x",
            "shopt -s extglob\nc=$'\\x14'\nw=\"a${c}xb\"\nprintf '%s' \"${w//*(x)/Q}\"\n",
            "QaQ\u{14}QQb",
        ),
        (
            "x_before_carrier",
            "shopt -s extglob\nc=$'\\x14'\nw=\"x${c}\"\nprintf '%s' \"${w//*(x)/Q}\"\n",
            "QQ\u{14}",
        ),
        (
            "two_carriers_then_x",
            "shopt -s extglob\nc=$'\\x14'\nw=\"${c}${c}x\"\nprintf '%s' \"${w//*(x)/Q}\"\n",
            "Q\u{14}Q\u{14}Q",
        ),
        (
            "two_kinds_of_carrier",
            "shopt -s extglob\nc=$'\\x14'\nd=$'\\x1f'\nw=\"${c}${d}\"\nprintf '%s' \"${w//*(x)/Q}\"\n",
            "Q\u{14}Q\u{1f}",
        ),
        (
            "carrier_mid_two_kinds",
            "shopt -s extglob\nc=$'\\x14'\nd=$'\\x1f'\nw=\"a${c}b${d}\"\nprintf '%s' \"${w//*(x)/Q}\"\n",
            "QaQ\u{14}QbQ\u{1f}",
        ),
        (
            "question_extglob",
            "shopt -s extglob\nc=$'\\x14'\nw=\"a${c}b\"\nprintf '%s' \"${w//?(x)/Q}\"\n",
            "QaQ\u{14}Qb",
        ),
        (
            "question_extglob_lone",
            "shopt -s extglob\nc=$'\\x14'\nw=\"$c\"\nprintf '%s' \"${w//?(x)/Q}\"\n",
            "Q\u{14}",
        ),
        (
            "single_slash_mid",
            "shopt -s extglob\nc=$'\\x14'\nw=\"a${c}b\"\nprintf '%s' \"${w/*(x)/Q}\"\n",
            "Qa\u{14}b",
        ),
        (
            "single_slash_lone",
            "shopt -s extglob\nc=$'\\x14'\nw=\"$c\"\nprintf '%s' \"${w/*(x)/Q}\"\n",
            "Q\u{14}",
        ),
        (
            "ascii_guard",
            "shopt -s extglob\nw=\"ab\"\nprintf '%s' \"${w//*(x)/Q}\"\n",
            "QaQb",
        ),
    ] {
        let (out, err, code) = run_file(script, tag);
        assert_eq!(code, 0, "{tag}: exit {code}, stderr {err}");
        assert_eq!(out, expected, "{tag}: stdout mismatch");
    }
}

/// Item 1 on the literal-char escape pair: a zero-length match landing
/// on a stored literal U+E000 (U+E400 + U+E000 transport pair) must
/// advance past the whole pair, not strand `cursor` between E400 and
/// its payload — same phantom-trailing-Q shape as the carrier pair.
#[test]
fn zero_length_match_advance_does_not_split_literal_pua_pair() {
    let (out, err, code) = run_file(
        "shopt -s extglob\nx=$'\\ue000'\nprintf '%s' \"${x//*(x)/Q}\"\n",
        "pua_pair",
    );
    assert_eq!(code, 0, "exit {code}, stderr {err}");
    assert_eq!(out, "Q\u{e000}", "stdout mismatch");
}

/// Item 2: a literal U+E000 in user data is ONE ordinary data
/// character in the replace face — `?`, `[...]`, a literal pattern,
/// `#`/`%` removal and `*` all treat it as a single unit, and
/// `${#w}` counts it as one.
#[test]
fn literal_e000_matches_as_one_data_char() {
    // (tag, script, expected stdout)
    for (tag, script, expected) in [
        (
            "question_lone",
            "w=$'\\ue000'\nprintf '%s' \"${w//?/_}\"\n",
            "_",
        ),
        ("len_lone", "w=$'\\ue000'\nprintf '%s' \"${#w}\"\n", "1"),
        (
            "prefix_question",
            "w=$'\\ue000'\nprintf '%s' \"${w#?}\"\n",
            "",
        ),
        (
            "suffix_question",
            "w=$'\\ue000'\nprintf '%s' \"${w%?}\"\n",
            "",
        ),
        (
            "bracket_lone",
            "w=$'\\ue000'\nprintf '%s' \"${w//[$'\\ue000']/_}\"\n",
            "_",
        ),
        (
            "literal_pattern_lone",
            "w=$'\\ue000'\nprintf '%s' \"${w//$'\\ue000'/_}\"\n",
            "_",
        ),
        (
            "star_lone",
            "w=$'\\ue000'\nprintf '%s' \"${w//*/_}\"\n",
            "_",
        ),
        (
            "question_mid",
            "w=$'\\ue000'\nv=\"a${w}b\"\nprintf '%s' \"${v//?/_}\"\n",
            "___",
        ),
        (
            "bracket_mid",
            "w=$'\\ue000'\nv=\"a${w}b\"\nprintf '%s' \"${v//[$'\\ue000']/_}\"\n",
            "a_b",
        ),
        (
            "literal_pattern_mid",
            "w=$'\\ue000'\nv=\"a${w}b\"\nprintf '%s' \"${v//$'\\ue000'/_}\"\n",
            "a_b",
        ),
        (
            "len_mid",
            "w=$'\\ue000'\nv=\"a${w}b\"\nprintf '%s' \"${#v}\"\n",
            "3",
        ),
        (
            "question_two",
            "u=$'\\ue000\\ue000'\nprintf '%s' \"${u//?/_}\"\n",
            "__",
        ),
        (
            "len_two",
            "u=$'\\ue000\\ue000'\nprintf '%s' \"${#u}\"\n",
            "2",
        ),
        (
            "bracket_two",
            "u=$'\\ue000\\ue000'\nprintf '%s' \"${u//[$'\\ue000']/_}\"\n",
            "__",
        ),
        (
            "prefix_bracket",
            "w=$'\\ue000'\nprintf '%s' \"${w#[$'\\ue000']}\"\n",
            "",
        ),
        (
            "suffix_bracket",
            "w=$'\\ue000'\nprintf '%s' \"${w%[$'\\ue000']}\"\n",
            "",
        ),
    ] {
        let (out, err, code) = run_file(script, tag);
        assert_eq!(code, 0, "{tag}: exit {code}, stderr {err}");
        assert_eq!(out, expected, "{tag}: stdout mismatch");
    }
}

/// Item 2, hard shape: a literal U+E000 immediately followed by a
/// payload-range PUA char (U+E001) must NOT fuse into one match unit
/// — the two data chars stay two units for `?`, `[...]` and `${#}`.
#[test]
fn literal_e000_followed_by_payload_range_char_stays_two_units() {
    // (tag, script, expected stdout)
    for (tag, script, expected) in [
        (
            "question",
            "w=$'\\ue000\\ue001'\nprintf '%s' \"${w//?/_}\"\n",
            "__",
        ),
        ("len", "w=$'\\ue000\\ue001'\nprintf '%s' \"${#w}\"\n", "2"),
        (
            "bracket_first",
            "w=$'\\ue000\\ue001'\nprintf '%s' \"${w//[$'\\ue000']/_}\"\n",
            "_\u{e001}",
        ),
        (
            "bracket_second",
            "w=$'\\ue000\\ue001'\nprintf '%s' \"${w//[$'\\ue001']/_}\"\n",
            "\u{e000}_",
        ),
        (
            "prefix_question",
            "w=$'\\ue000\\ue001'\nprintf '%s' \"${w#?}\"\n",
            "\u{e001}",
        ),
        (
            "suffix_question",
            "w=$'\\ue000\\ue001'\nprintf '%s' \"${w%?}\"\n",
            "\u{e000}",
        ),
    ] {
        let (out, err, code) = run_file(script, tag);
        assert_eq!(code, 0, "{tag}: exit {code}, stderr {err}");
        assert_eq!(out, expected, "{tag}: stdout mismatch");
    }
}

/// Item 2 across entry paths: command-substitution bytes and a literal
/// U+E000 typed in script source must reach the replace face as the
/// same one data character as `$'\ue000'`.
#[test]
fn literal_e000_from_other_entries_matches_as_one_char() {
    // Command substitution: printf emits the raw UTF-8 bytes.
    let (out, err, code) = run_file(
        "w=$(printf '\\U0000E000')\nprintf '%s|%s' \"${w//?/_}\" \"${#w}\"\n",
        "comsub",
    );
    assert_eq!(code, 0, "comsub: exit {code}, stderr {err}");
    assert_eq!(out, "_|1", "comsub: stdout mismatch");

    // Literal U+E000 typed in the script source itself (raw bytes in
    // the file, not an escape): "a<U+E000>b" is three data chars.
    let e000 = '\u{e000}';
    let script = format!("w=\"a{e000}b\"\nprintf '%s|%s' \"${{w//?/_}}\" \"${{#w}}\"\n");
    let (out, err, code) = run_file(&script, "source_literal");
    assert_eq!(code, 0, "source_literal: exit {code}, stderr {err}");
    assert_eq!(out, "___|3", "source_literal: stdout mismatch");
}

/// Item 2, storage face: `printf -v` output is DATA — a literal
/// U+E000 in it must reach storage in value-entry spelling, so the
/// value keeps its two data chars in every face (GNU: len 2, replace
/// `__`, plain expansion the two PUA chars, `declare -p` the
/// double-quoted pair). Before the fix the formatted text was stored
/// raw and every boundary folded U+E000 U+E001 into one NUL byte.
#[test]
fn printf_v_stores_literal_pua_chars_as_data() {
    // (tag, script, expected stdout)
    for (tag, script, expected) in [
        (
            "len",
            "printf -v v '\\uE000\\uE001'\nprintf '%s' \"${#v}\"\n",
            "2",
        ),
        (
            "replace",
            "printf -v v '\\uE000\\uE001'\nprintf '%s' \"${v//?/_}\"\n",
            "__",
        ),
        (
            "plain",
            "printf -v v '\\uE000\\uE001'\nprintf '%s' \"$v\"\n",
            "\u{e000}\u{e001}",
        ),
        (
            "declare_p",
            "printf -v v '\\uE000\\uE001'\ndeclare -p v\n",
            "declare -- v=\"\u{e000}\u{e001}\"\n",
        ),
        (
            "lone_len",
            "printf -v u '\\uE000'\nprintf '%s' \"${#u}\"\n",
            "1",
        ),
        (
            "lone_declare_p",
            "printf -v u '\\uE000'\ndeclare -p u\n",
            "declare -- u=\"\u{e000}\"\n",
        ),
        (
            // rubash#3921 (D2 fallout): a `%s` argument is transport
            // text already — the -v store must round-trip it, not
            // re-encode it (entry_encode_value_text double-escaped the
            // pairs, so the PUA chars stored as four chars and the
            // byte as two). GNU: len 2, declare -p the pair.
            "pct_s_pua_len",
            "w2=$'\\ue000\\ue001'\nprintf -v v '%s' \"$w2\"\nprintf '%s' \"${#v}\"\n",
            "2",
        ),
        (
            "pct_s_pua_declare",
            "w2=$'\\ue000\\ue001'\nprintf -v v '%s' \"$w2\"\ndeclare -p v\n",
            "declare -- v=\"\u{e000}\u{e001}\"\n",
        ),
        (
            "pct_s_byte_len",
            "c=$'\\x14'\nprintf -v b '%s' \"$c\"\nprintf '%s' \"${#b}\"\n",
            "1",
        ),
        (
            "pct_s_byte_declare",
            "c=$'\\x14'\nprintf -v b '%s' \"$c\"\ndeclare -p b\n",
            "declare -- b=$'\\024'\n",
        ),
    ] {
        let (out, err, code) = run_file(script, tag);
        assert_eq!(code, 0, "{tag}: exit {code}, stderr {err}");
        assert_eq!(out, expected, "{tag}: stdout mismatch");
    }
}
/// Item 2, display face: `declare -p` (and the array/assoc
/// element renders) must show a stored literal U+E000 U+E001 as the
/// double-quoted pair, not fold it into `$'\000'` (GNU:
/// `declare -- w2="<E000><E001>"`). Before the fix the display
/// quoter re-decoded the visible text's lone U+E000 as a raw-byte
/// marker pair.
#[test]
fn declare_p_shows_literal_pua_chars_as_data() {
    // (tag, script, expected stdout)
    for (tag, script, expected) in [
        (
            "scalar",
            "w=$'\\ue000\\ue001'\ndeclare -p w\n",
            "declare -- w=\"\u{e000}\u{e001}\"\n",
        ),
        (
            "scalar_lone",
            "w=$'\\ue000'\ndeclare -p w\n",
            "declare -- w=\"\u{e000}\"\n",
        ),
        (
            "array_element",
            "a=($'\\ue000\\ue001')\ndeclare -p a\n",
            "declare -a a=([0]=\"\u{e000}\u{e001}\")\n",
        ),
        (
            "assoc_value",
            "declare -A h=( [k]=$'\\ue000\\ue001' )\ndeclare -p h\n",
            "declare -A h=([k]=\"\u{e000}\u{e001}\" )\n",
        ),
        (
            // rubash#3921 array fold, other arm of the selective
            // decode: a typed registry-zone char OUTSIDE the raw-byte
            // range (U+E314) must still resolve to the lone char —
            // the flush only decodes when U+E000 is present, so an
            // unresolved `E400 + E314` would print the introducer.
            "array_e314",
            "e=($'\\uE314')\ndeclare -p e\n",
            "declare -a e=([0]=\"\u{e314}\")\n",
        ),
        (
            // Mixed array: PUA pair, genuine control byte, lone E000.
            // The byte keeps its `$'\024'` octal form; the lone E000
            // keeps its pair through the fold and the flush.
            "array_mixed",
            "m=($'\\ue000\\ue001' $'\\x14' $'\\ue000')\ndeclare -p m\n",
            "declare -a m=([0]=\"\u{e000}\u{e001}\" [1]=$'\\024' [2]=\"\u{e000}\")\n",
        ),
    ] {
        let (out, err, code) = run_file(script, tag);
        assert_eq!(code, 0, "{tag}: exit {code}, stderr {err}");
        assert_eq!(out, expected, "{tag}: stdout mismatch");
    }
}

/// Item 2, format-escape face (D2): `printf '\uE000\uE001'` and
/// `printf '%b' '\uE000\uE001'` must emit the two PUA chars, not fold
/// them to NUL. Before the fix `unicode_escape_text` returned the lone
/// chars as visible text and the printf output path's `raw_bytes`
/// folded them — this fired even through a pipe, unlike the direct
/// flush double-decode (D1), which the `plain` cell in the `-v` test
/// above already locks.
#[test]
fn printf_format_unicode_escape_preserves_literal_pua_chars() {
    // (tag, script, expected stdout)
    for (tag, script, expected) in [
        (
            "fmt_direct",
            "printf '\\uE000\\uE001'\n",
            "\u{e000}\u{e001}",
        ),
        (
            "pct_b_direct",
            "printf '%b' '\\uE000\\uE001'\n",
            "\u{e000}\u{e001}",
        ),
    ] {
        let (out, err, code) = run_file(script, tag);
        assert_eq!(code, 0, "{tag}: exit {code}, stderr {err}");
        assert_eq!(out, expected, "{tag}: stdout mismatch");
    }
}

/// Item 2, @Q round-trip face: `${var@Q}` renders the quoted form as
/// transport text, so assigning it stores the 4-char `'...'` form
/// (GNU: len 4) instead of folding the lone U+E000 + payload pair to
/// a NUL byte at the assignment boundary (quote+NUL+quote, len 3).
#[test]
fn at_q_round_trip_preserves_literal_pua_chars() {
    // (tag, script, expected stdout)
    for (tag, script, expected) in [
        (
            "q_len",
            "w2=$'\\ue000\\ue001'\nq=\"${w2@Q}\"\nprintf '%s' \"${#q}\"\n",
            "4",
        ),
        (
            "q_declare_p",
            "w2=$'\\ue000\\ue001'\nq=\"${w2@Q}\"\ndeclare -p q\n",
            "declare -- q=\"'\u{e000}\u{e001}'\"\n",
        ),
        (
            "q_direct",
            "w2=$'\\ue000\\ue001'\nprintf '%s' \"${w2@Q}\"\n",
            "'\u{e000}\u{e001}'",
        ),
        (
            "array_q_len",
            "a=($'\\ue000\\ue001')\nc=\"${a[0]@Q}\"\nprintf '%s' \"${#c}\"\n",
            "4",
        ),
        (
            "array_q_declare_p",
            "a=($'\\ue000\\ue001')\nc=\"${a[0]@Q}\"\ndeclare -p c\n",
            "declare -- c=\"'\u{e000}\u{e001}'\"\n",
        ),
    ] {
        let (out, err, code) = run_file(script, tag);
        assert_eq!(code, 0, "{tag}: exit {code}, stderr {err}");
        assert_eq!(out, expected, "{tag}: stdout mismatch");
    }
}
