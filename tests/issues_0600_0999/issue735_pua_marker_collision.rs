//! Issue rubash#735: user-typed PUA codepoints collide with the marker
//! registry.
//!
//! Two defects, GNU bash 5.3 baseline (UTF-8 locale):
//! 1. `$'\uE401'` was silently swallowed (empty value, rc=0, no diagnostic).
//!    U+E401 is the internal ANSI_C_IFS_GUARD marker; a user typing that
//!    codepoint through ordinary ANSI-C quoting got nothing back.
//! 2. `${#x}` miscounted PUA codepoints (`$'\uE301'`/`$'\uE314'` gave 2,
//!    not 1) even though the stored bytes were byte-identical to GNU.
//!
//! Root cause: `markers.rs` documents that `user_reachable` markers must be
//! re-encoded at entry, but ANSI_C_IFS_GUARD (U+E401) lived outside the
//! marker zone (E000..=E3FF), so `push_literal_char` never E400-escaped it.
//! And `parameter_char_length` counted the E400 escape introducer as a
//! character because it only decoded raw-byte (E000) pairs.
//!
//! Fix:
//! - `MARKER_ZONE_LAST` extended to 0xE401 so user U+E401 is E400-escaped
//!   at entry (ANSI-C `\u` decode, `bytes_to_shell_text`).
//! - All ANSI_C_IFS_GUARD strips/conversions are E400-aware
//!   (`strip_ansi_c_ifs_guard`, `ansi_c_ifs_guard_to_ifs_glue`).
//! - `parameter_char_length` decodes the E400 escape before counting.
//!
//! Expected outputs are byte-verified against GNU bash (script-file probes).

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

/// Defect 1: `$'\uE401'` must survive as one char, not be swallowed.
#[test]
fn ansi_c_e401_not_swallowed() {
    let (stdout, stderr, _) =
        rubash("x2=$'\\uE401'; printf 'len=%s\\n' \"${#x2}\"; printf '%s' \"$x2\" | od -A n -t x1");
    assert_clean(&stderr);
    assert!(stdout.contains("len=1"), "expected len=1, got: {stdout}");
    assert!(
        stdout.contains("ee 90 81"),
        "expected U+E401 bytes ee 90 81, got: {stdout}"
    );
}

/// Defect 2: `${#}` counts user PUA chars as 1, not 2.
#[test]
fn hash_counts_pua_as_one() {
    let (stdout, stderr, _) = rubash(
        "x1=$'\\uE301'; printf 'len=%s\\n' \"${#x1}\"; x3=$'\\uE314'; printf 'len=%s\\n' \"${#x3}\"",
    );
    assert_clean(&stderr);
    assert_eq!(stdout, "len=1\nlen=1\n", "unexpected: {stdout}");
}

/// The guard's original job still works: decoded `$'...'` blanks stay
/// literal word data through field splitting (rubash#379).
#[test]
fn guard_still_protects_decoded_blanks() {
    let (stdout, stderr, _) = rubash("v=$'a b'; printf '[%s]\\n' \"$v\"; printf '[%s]\\n' a\\ b");
    assert_clean(&stderr);
    assert_eq!(stdout, "[a b]\n[a b]\n", "unexpected: {stdout}");
}

/// User U+E401 alongside a real decoded blank: both survive correctly.
#[test]
fn user_e401_with_real_guard() {
    let (stdout, stderr, _) =
        rubash("m=$'a\\uE401 b'; printf 'len=%s\\n' \"${#m}\"; printf '%s' \"$m\" | od -A n -t x1");
    assert_clean(&stderr);
    assert!(stdout.contains("len=4"), "expected len=4, got: {stdout}");
    // a (61) + U+E401 (ee 90 81) + space (20) + b (62)
    assert!(
        stdout.contains("61 ee 90 81 20 62"),
        "expected a + U+E401 + space + b, got: {stdout}"
    );
}

/// The E400 escape introducer itself round-trips as user data.
#[test]
fn e400_literal_roundtrips() {
    let (stdout, stderr, _) =
        rubash("e=$'\\uE400'; printf 'len=%s\\n' \"${#e}\"; printf '%s' \"$e\" | od -A n -t x1");
    assert_clean(&stderr);
    assert!(stdout.contains("len=1"), "expected len=1, got: {stdout}");
    assert!(
        stdout.contains("ee 90 80"),
        "expected U+E400 bytes ee 90 80, got: {stdout}"
    );
}
