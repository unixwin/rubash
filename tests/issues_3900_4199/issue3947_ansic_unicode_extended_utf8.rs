//! rubash#3947 — `$'\U........'` values in the band
//! U+110000..=U+7FFFFFFF stayed the literal 10 characters `\U00XXXXXX`
//! under a UTF-8 locale. GNU encodes them with the pre-2003 extended
//! UTF-8 scheme (4/5/6 bytes) through the same u32cconv path that
//! already handled surrogates and valid scalars; only the
//! >= 0x80000000 band produces nothing, in every locale.
//!
//! GNU owners:
//!  - lib/sh/strtrans.c:161-181 ansicstr: the `\u`/`\U` branch hands
//!    every value > 0x7f to u32cconv.
//!  - lib/sh/unicode.c:239 u32cconv -> unicode.c:154 u32toutf8: the
//!    <0x200000 / <0x04000000 / <0x080000000 length classes emit the
//!    4/5/6-byte forms; >= 0x080000000 emits zero bytes. In a non-UTF-8
//!    locale the iconv leg fails and u32tocesc (unicode.c:141,
//!    `\u%04X` / `\U%08X`, uppercase hex) keeps the escape literal.
//!
//! Oracle: WSL /usr/local/bin/bash (GNU Bash 5.3.x built from the
//! vendored tree), script-file probes 2026-11-21 under LC_ALL=C.UTF-8
//! and LC_ALL=C — byte-identical to the expectations below, including
//! the band boundaries U+1FFFFF / U+200000 / U+3FFFFFF / U+4000000 and
//! the mid-word/adjacent-escape compositions.
//!
//! Length face (fixed by rubash#4035): `${#v}` counts the extended
//! forms as ONE character — glibc's `mbrlen` accepts the pre-2003
//! 4/5/6-byte range where Rust's UTF-8 decoder rejects it — see
//! `utf8_mblen`/`multibyte_char_count` in src/locale.rs.

use std::process::Command;

fn rubash(script: &str, locale: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", locale)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn utf8_locale_encodes_extended_band() {
    // One od line per escape, band boundaries included: 4-byte forms
    // top out at U+1FFFFF, 5-byte at U+3FFFFFF, 6-byte at U+7FFFFFFF.
    let (stdout, stderr, code) = rubash(
        "v=$'\\U00110000'; printf '%s' \"$v\" | od -An -tx1\n\
         v=$'\\U001FFFFF'; printf '%s' \"$v\" | od -An -tx1\n\
         v=$'\\U00200000'; printf '%s' \"$v\" | od -An -tx1\n\
         v=$'\\U03FFFFFF'; printf '%s' \"$v\" | od -An -tx1\n\
         v=$'\\U04000000'; printf '%s' \"$v\" | od -An -tx1\n\
         v=$'\\U7FFFFFFF'; printf '%s' \"$v\" | od -An -tx1",
        "C.UTF-8",
    );
    assert_eq!(
        stdout,
        " f4 90 80 80\n f7 bf bf bf\n f8 88 80 80 80\n fb bf bf bf bf\n fc 84 80 80 80 80\n fd bf bf bf bf bf\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn utf8_locale_argument_position_encodes_extended_band() {
    // The escape as a bare command argument (not only assignment RHS):
    // echo/printf operands take the same ansicstr decode.
    let (stdout, stderr, code) = rubash(
        "echo $'\\U00110000' | od -An -tx1\n\
         echo x$'\\U00200000'y | od -An -tx1\n\
         printf '%s' $'\\U7FFFFFFF' | od -An -tx1",
        "C.UTF-8",
    );
    assert_eq!(
        stdout,
        " f4 90 80 80 0a\n 78 f8 88 80 80 80 79 0a\n fd bf bf bf bf bf\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn utf8_locale_extended_band_controls() {
    // Surrogate stays CESU-8, U+10FFFF/U+10000/\\u00e9 keep their
    // encodings, >= 0x80000000 stays empty, and the extended bytes
    // compose mid-word and adjacent to each other like GNU.
    let (stdout, stderr, code) = rubash(
        "v=$'\\U0000D800'; printf '%s' \"$v\" | od -An -tx1\n\
         v=$'\\U0010FFFF'; printf '%s' \"$v\" | od -An -tx1\n\
         v=$'\\U00010000'; printf '%s' \"$v\" | od -An -tx1\n\
         v=$'\\u00e9'; printf '%s' \"$v\" | od -An -tx1\n\
         v=$'\\U80000000'; echo \"len80000000=${#v}\"; printf '%s' \"$v\" | od -An -tx1; echo end\n\
         v=$'\\UFFFFFFFF'; echo \"lenFFFFFFFF=${#v}\"\n\
         x=a$'\\U00110000'b; printf '%s' \"$x\" | od -An -tx1\n\
         y=$'\\U00110000\\U00200000'; printf '%s' \"$y\" | od -An -tx1",
        "C.UTF-8",
    );
    assert_eq!(
        stdout,
        concat!(
            " ed a0 80\n f4 8f bf bf\n f0 90 80 80\n c3 a9\n",
            "len80000000=0\nend\nlenFFFFFFFF=0\n",
            " 61 f4 90 80 80 62\n f4 90 80 80 f8 88 80 80 80\n",
        )
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn c_locale_keeps_extended_band_literal() {
    // Non-UTF-8 locale: u32cconv's iconv leg fails and u32tocesc
    // re-emits the escape (uppercase hex, `\\u%04X` below U+10000,
    // `\\U%08X` at or above it). >= 0x80000000 is empty here too.
    let (stdout, stderr, code) = rubash(
        "v=$'\\U00110000'; printf '%s' \"$v\"; echo\n\
         v=$'\\U7FFFFFFF'; printf '%s' \"$v\"; echo\n\
         v=$'\\U0000D800'; printf '%s' \"$v\"; echo\n\
         v=$'\\U0010FFFF'; printf '%s' \"$v\"; echo\n\
         v=$'\\U00010000'; printf '%s' \"$v\"; echo\n\
         v=$'\\UFFFFFFFF'; echo \"len=${#v}\"",
        "C",
    );
    assert_eq!(
        stdout,
        "\\U00110000\n\\U7FFFFFFF\n\\uD800\n\\U0010FFFF\n\\U00010000\nlen=0\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
