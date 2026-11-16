//! rubash#3619 — the $'\u'/'\U' decode gate consulted the REQUESTED
//! locale name, not the locale that survives a failed setlocale().
//!
//! GNU (locale.c set_locale_var -> the real setlocale call) is
//! stateful: a failed setlocale warns and leaves the previously
//! effective locale in force. So under an un-activatable name
//! (bogus_LOCALE / zh_TW.big5 on a box without those archives) the
//! surviving UTF-8 locale still decodes `\u3b1` to α, while an
//! ACTIVATABLE non-UTF-8 name (C) does impose byte semantics and the
//! escape re-emits literally.
//!
//! GNU bash 5.3.20 oracle (script-file probe, LC_ALL=C.UTF-8 in env):
//!   LC_ALL=zh_TW.big5; echo a $'\u3b1'   ->  a α
//!   LC_ALL=C;         echo d $'\u3b1'   ->  d \u03B1
//!
//! Pre-fix rubash emitted the literal `\u03B1` for the failed-locale
//! line because `locale_name_selects_utf8` returned false for any
//! non-UTF-8 name without checking whether it could activate.
#![cfg(all(unix, not(target_os = "android")))]

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

#[test]
fn unactivatable_name_keeps_surviving_utf8_locale() {
    let (stdout, stderr, code) = rubash(
        "LC_ALL=bogus_LOCALE\n\
         echo a $'\\u3b1'\n\
         echo B$'\\u03b1'B",
    );
    // The failed setlocale leaves the UTF-8 locale in force: α decodes.
    assert_eq!(stdout, "a \u{3b1}\nB\u{3b1}B\n");
    assert!(stderr.contains("cannot change locale"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn activatable_non_utf8_name_still_forces_bytes() {
    let (stdout, _, code) = rubash(
        "LC_ALL=bogus_LOCALE\n\
         echo a $'\\u3b1'\n\
         LC_ALL=C\n\
         echo d $'\\u3b1'",
    );
    // C is activatable: after it binds, byte semantics re-emit the
    // escape literally (GNU upper-cases the hex digits).
    assert_eq!(stdout, "a \u{3b1}\nd \\u03B1\n");
    assert_eq!(code, Some(0));
}
