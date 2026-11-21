//! rubash#4035 — `${#v}` and `${v:off:len}` counted the bytes of an
//! extended UTF-8 sequence (pre-2003 range, codepoints above U+10FFFF
//! up to U+7FFFFFFF: 4/5/6-byte forms such as `$'\xF4\x90\x80\x80'`,
//! `printf -v v '\U00110000'`, U+200000, U+7FFFFFFF) where GNU bash
//! 5.3.20 counts ONE character.
//!
//! GNU owners:
//!  - `${#name}` -> parameter_brace_expand_length (subst.c:8258) ->
//!    MB_STRLEN (subst.c:8308/8323/8332) -> MBSLEN
//!    (include/shmbutil.h:48-49) -> mbstrlen (lib/sh/shmbchar.c:56-79),
//!    which steps with the C library's `mbrlen`.
//!  - `${var:off:len}` -> parameter_brace_substring (subst.c:9050) ->
//!    mb_substring (subst.c:9022) -> ADVANCE_CHAR
//!    (include/shmbutil.h:114-147), the same `mbrlen` walk.
//!
//! glibc's UTF-8 decoder accepts sequences of up to six bytes encoding
//! up to U+7FFFFFFF — wider than Rust's `str` UTF-8 (scalar values,
//! four bytes max) — while still rejecting overlong forms, the
//! surrogate band, and lone continuation bytes per byte. rubash's
//! `utf8_mblen`/`multibyte_char_count`/`multibyte_char_offsets`
//! (src/locale.rs) now mirror that walk; the expectations below are
//! byte-identical to WSL /usr/local/bin/bash (GNU Bash 5.3.20) probes
//! of 2026-11-21 under LC_ALL=C.utf8 and LC_ALL=C.

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
fn hash_length_counts_extended_utf8_as_one_character() {
    // Every extended form GNU's mbrlen accepts: 4-byte past U+10FFFF
    // (F4 second >= 0x90, F5..=F7 leads), 5-byte F8..=FB, 6-byte
    // FC..=FD — each is ONE character, not its byte count.
    let (stdout, stderr, code) = rubash(
        "v=$'\\xf4\\x90\\x80\\x80'; echo \"u110000=${#v}\"\n\
         v=$'\\xf5\\x80\\x80\\x80'; echo \"u140000=${#v}\"\n\
         v=$'\\xf7\\xbf\\xbf\\xbf'; echo \"u1fffff=${#v}\"\n\
         v=$'\\xf8\\x88\\x80\\x80\\x80'; echo \"u200000=${#v}\"\n\
         v=$'\\xfb\\xbf\\xbf\\xbf\\xbf'; echo \"u3ffffff=${#v}\"\n\
         v=$'\\xfc\\x84\\x80\\x80\\x80\\x80'; echo \"u4000000=${#v}\"\n\
         v=$'\\xfd\\xbf\\xbf\\xbf\\xbf\\xbf'; echo \"u7fffffff=${#v}\"",
        "C.UTF-8",
    );
    assert_eq!(
        stdout,
        "u110000=1\nu140000=1\nu1fffff=1\nu200000=1\nu3ffffff=1\nu4000000=1\nu7fffffff=1\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn hash_length_extended_utf8_via_printf_and_mixed() {
    // The issue's producer forms: printf -v / \\U escapes, and the
    // extended character embedded in wider values.
    let (stdout, stderr, code) = rubash(
        "printf -v v '\\U00110000'; echo \"printfU110000=${#v}\"\n\
         printf -v w '\\U7FFFFFFF'; echo \"printfU7fffffff=${#w}\"\n\
         v=$'a\\xf4\\x90\\x80\\x80'; echo \"a+ext=${#v}\"\n\
         v=$'\\xf4\\x90\\x80\\x80a'; echo \"ext+a=${#v}\"\n\
         v=$'\\xc3\\xa9\\xf4\\x90\\x80\\x80'; echo \"eacute+ext=${#v}\"\n\
         v=$(printf '\\U00200000'); echo \"cmdsub=${#v}\"",
        "C.UTF-8",
    );
    assert_eq!(
        stdout,
        "printfU110000=1\nprintfU7fffffff=1\na+ext=2\next+a=2\neacute+ext=2\ncmdsub=1\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn hash_length_rejects_overlong_surrogate_and_truncated() {
    // glibc mbrlen still rejects what UTF-8 forbids: overlong leads
    // (C0/C1, E0 <A0, F0 <90, F8 <88, FC <84), the surrogate band
    // (ED >=A0), lone continuations, and truncated sequences — each
    // rejected byte is one character.
    let (stdout, stderr, code) = rubash(
        "v=$'\\xed\\xa0\\x80'; echo \"surrogateD800=${#v}\"\n\
         v=$'\\xed\\xbf\\xbf'; echo \"surrogateDFFF=${#v}\"\n\
         v=$'\\xed\\x9f\\xbf'; echo \"belowSurrogate=${#v}\"\n\
         v=$'\\xc0\\x80'; echo \"overlongC0=${#v}\"\n\
         v=$'\\xe0\\x9f\\xbf'; echo \"overlongE0=${#v}\"\n\
         v=$'\\xf0\\x8f\\x80\\x80'; echo \"overlongF0=${#v}\"\n\
         v=$'\\xf8\\x87\\xbf\\xbf\\xbf'; echo \"overlongF8=${#v}\"\n\
         v=$'\\xfc\\x83\\xbf\\xbf\\xbf\\xbf'; echo \"overlongFC=${#v}\"\n\
         v=$'\\xff'; echo \"loneFF=${#v}\"\n\
         v=$'\\xf4\\x90'; echo \"truncated=${#v}\"\n\
         v=$'\\xf4\\x90\\x41'; echo \"badCont=${#v}\"\n\
         v=$'a\\xffb'; echo \"mixedInvalid=${#v}\"",
        "C.UTF-8",
    );
    assert_eq!(
        stdout,
        concat!(
            "surrogateD800=3\nsurrogateDFFF=3\nbelowSurrogate=1\n",
            "overlongC0=2\noverlongE0=3\noverlongF0=4\noverlongF8=5\n",
            "overlongFC=6\nloneFF=1\ntruncated=2\nbadCont=3\nmixedInvalid=3\n",
        )
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn substring_steps_over_extended_utf8_as_one_character() {
    // ${v:off:len} shares the same mbrlen walk (mb_substring,
    // subst.c:9022): one offset step covers the whole extended
    // sequence, so the slice returns all of its bytes.
    let (stdout, stderr, code) = rubash(
        "v=$'\\xf4\\x90\\x80\\x80'; printf '%s' \"${v:0:1}\" | od -An -tx1\n\
         v=$'ab\\xf4\\x90\\x80\\x80cd'; printf '%s' \"${v:2:1}\" | od -An -tx1\n\
         printf '%s' \"${v:2:2}\" | od -An -tx1\n\
         printf '%s' \"${v:3:1}\" | od -An -tx1\n\
         v=$'\\xf8\\x88\\x80\\x80\\x80'; printf '%s' \"${v:0:1}\" | od -An -tx1",
        "C.UTF-8",
    );
    assert_eq!(
        stdout,
        " f4 90 80 80\n f4 90 80 80\n f4 90 80 80 63\n 63\n f8 88 80 80 80\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn c_locale_counts_extended_utf8_as_bytes() {
    // Single-byte locale: MB_CUR_MAX is 1, every byte is a character —
    // the extended sequence counts its 4/5/6 bytes.
    let (stdout, stderr, code) = rubash(
        "v=$'\\xf4\\x90\\x80\\x80'; echo \"u110000=${#v}\"\n\
         v=$'\\xf8\\x88\\x80\\x80\\x80'; echo \"u200000=${#v}\"\n\
         v=$'\\xfd\\xbf\\xbf\\xbf\\xbf\\xbf'; echo \"u7fffffff=${#v}\"\n\
         v=$'\\xc3\\xa9'; echo \"eacute=${#v}\"",
        "C",
    );
    assert_eq!(stdout, "u110000=4\nu200000=5\nu7fffffff=6\neacute=2\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
