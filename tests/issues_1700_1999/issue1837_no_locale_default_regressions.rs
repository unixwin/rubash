//! rubash#1837 — with NO locale variable set anywhere (no LANG, no
//! LC_*), the `\u`/`\U` decoder gates defaulted to UTF-8 while the
//! `${#var}` length gate defaulted to single-byte counting, so the two
//! faces disagreed inside one environment: `$'\uE301'` decoded to a
//! 3-byte character whose length then counted as 3.
//!
//! GNU bash 5.3.20 resolves the locale with setlocale(LC_ALL, "")
//! (locale.c set_default_locale): no variables means the C locale, in
//! BOTH faces — the decoder keeps `\uXXXX` literal for values > 0x7f
//! (lib/sh/strtrans.c:161-181 -> lib/sh/unicode.c:239 u32cconv, fallback
//! u32tocesc `\u%04X`/`\U%08X`) and `${#var}` counts bytes.
//!
//! Expectations probed against GNU bash 5.3.20 with `env -i` (script
//! forms below). The child is spawned with a cleared environment so the
//! no-locale shape holds regardless of the test runner's own locale;
//! explicit-locale spawns are the control cells (they already agreed
//! with GNU before this fix and must not regress).

use std::process::Command;

fn rubash_with_env(script: &str, extra_env: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .arg("-c")
        .arg(script);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// No locale variables at all: the environment of the issue's probe.
fn rubash_no_locale(script: &str) -> (String, String, Option<i32>) {
    rubash_with_env(script, &[])
}

#[test]
fn issue1837_no_locale_dollar_quote_u_stays_literal_and_counts_bytes() {
    // GNU (env -i): x=$'\uE301' -> the six literal characters, ${#x} = 6;
    // same for \u00e9 and \u4e2d. Before the fix the value decoded to a
    // 3-byte character and ${#x} reported 3.
    let (stdout, stderr, code) = rubash_no_locale(
        "x=$'\\uE301'; echo ${#x}; printf '%s' \"$x\"; echo; \
         y=$'\\u00e9'; echo ${#y}; z=$'\\u4e2d'; echo ${#z}",
    );
    assert_eq!(stdout, "6\n\\uE301\n6\n6\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1837_no_locale_ascii_u_still_decodes_and_mixed_run_is_coherent() {
    // strtrans.c:174: values <= 0x7f translate directly in every locale,
    // so \u0041 is "A" even with no locale — and a mixed run counts the
    // decoded ASCII char plus the literal escape: 1 + 6 = 7 (GNU).
    let (stdout, stderr, code) =
        rubash_no_locale("x=$'\\u0041'; echo \"$x ${#x}\"; y=$'\\u0041\\u00e9'; echo ${#y}");
    assert_eq!(stdout, "A 1\n7\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1837_no_locale_echo_e_u_stays_literal() {
    // echo -e shares the decoder (echo.def -> ansicstr): with no locale
    // variables GNU prints the escapes literally, hex upper-cased.
    let (stdout, stderr, code) = rubash_no_locale("echo -e '\\u00e9'; echo -e '\\U0001F600'");
    assert_eq!(stdout, "\\u00E9\n\\U0001F600\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1837_explicit_c_locale_matches_the_no_locale_default() {
    // Control cell: LC_ALL=C spelled out must behave exactly like the
    // no-variable default (this already held before the fix).
    let (stdout, stderr, code) = rubash_with_env(
        "x=$'\\uE301'; echo ${#x}; echo -e '\\u00e9'",
        &[("LC_ALL", "C")],
    );
    assert_eq!(stdout, "6\n\\u00E9\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1837_explicit_utf8_locale_still_decodes() {
    // Control cell: an active UTF-8 locale decodes in both faces —
    // ${#x} counts the one decoded character and echo -e emits its
    // UTF-8 bytes (c3 a9). The fix must not close the gate here.
    let (stdout, stderr, code) = rubash_with_env(
        "x=$'\\uE301'; echo ${#x}; echo -e '\\u00e9'",
        &[("LC_ALL", "C.UTF-8")],
    );
    assert_eq!(stdout, "1\n\u{e9}\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
