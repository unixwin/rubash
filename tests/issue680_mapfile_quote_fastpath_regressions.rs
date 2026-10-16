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
