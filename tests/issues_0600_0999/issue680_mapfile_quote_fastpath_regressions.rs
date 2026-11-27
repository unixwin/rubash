//! rubash#680 (mapfile store-side): `quote_array_value` fast-path contract.
//!
//! GNU array.c:948-989 `array_to_assign` wraps elements via `ansic_quote`
//! only when `ansic_shouldquote` fires (strtrans.c:341) and via
//! `sh_double_quote` (shquote.c:134, escapes only `\`$"\n`) otherwise, so a
//! printable-ASCII value outside that escape set renders as `"value"`
//! verbatim. The fast path must be byte-identical to the full path, and any
//! value outside the whitelist falls through unchanged (rubash#117).

use std::process::Command;

fn rubash(source: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", source])
        .output()
        .expect("run rubash");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Plain values (the `seq` bench shape) render double-quoted verbatim.
#[test]
fn mapfile_plain_values_render_verbatim() {
    assert_eq!(
        rubash("mapfile -t n <<< $'1\\n20\\n300'; declare -p n"),
        "declare -a n=([0]=\"1\" [1]=\"20\" [2]=\"300\")\n"
    );
}

/// Whitelisted punctuation and spaces take the fast path too.
#[test]
fn mapfile_fastpath_punctuation_and_space() {
    assert_eq!(
        rubash("mapfile -t w <<< $'a b\\nx/y:z\\na=b'; declare -p w"),
        "declare -a w=([0]=\"a b\" [1]=\"x/y:z\" [2]=\"a=b\")\n"
    );
}

/// Values needing escapes fall through to the full quoting path.
#[test]
fn mapfile_escape_values_fall_through() {
    assert_eq!(
        rubash("mapfile -t e <<< $'a$b\\na\"b\\na\\\\b\\na`b'; declare -p e"),
        "declare -a e=([0]=\"a\\$b\" [1]=\"a\\\"b\" [2]=\"a\\\\b\" [3]=\"a\\`b\")\n"
    );
}

/// rubash#680 slice-2 followup (rescue/stash-perf-680 fusion): multi-byte
/// UTF-8 payloads ride the encoded-split fast path — the ASCII delimiter can
/// never cut a multi-byte sequence and a valid UTF-8 segment re-encodes to
/// itself, so `中文` values round-trip byte-identically. GNU 5.3.0 probe
/// (2026-10-18): `declare -p` prints `[1]="中文 beta"` verbatim.
#[test]
fn mapfile_fastpath_rides_utf8_payloads() {
    assert_eq!(
        rubash("mapfile -t u <<< $'alpha\\n中文 beta\\ngamma'; declare -p u"),
        "declare -a u=([0]=\"alpha\" [1]=\"中文 beta\" [2]=\"gamma\")\n"
    );
}

/// The gate is the AUTHORITATIVE carrier set plus the 0xEE marker-zone lead:
/// a carrier byte or a registry marker inside the stream falls back to the
/// decode path (identical values, no fast-path corruption), while a
/// non-carrier control byte (tab) still rides the fast path.
#[test]
fn mapfile_fastpath_gate_carriers_and_marker_zone() {
    assert_eq!(
        rubash("mapfile -t c <<< $'a\\fb\\ne\\021f\\nz'; declare -p c"),
        "declare -a c=([0]=$'a\\fb' [1]=$'e\\021f' [2]=\"z\")\n"
    );
    assert_eq!(
        rubash("mapfile -t t <<< $'x\\ty\\nz'; declare -p t"),
        "declare -a t=([0]=$'x\\ty' [1]=\"z\")\n"
    );
}
