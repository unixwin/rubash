//! rubash#1488 — `echo -e` expanded \u/\U escapes even under
//! LC_ALL=C; GNU keeps them literal in the C locale (re-emitting the
//! escape with upper-cased hex digits) and expands them only in a
//! UTF-8 locale.
//!
//! Probe: clusters/x4/repro/1488.sh (asserted as od hex bytes so the
//! expectation is locale-text independent).
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn echo_e_unicode_escapes_respect_c_locale() {
    let (stdout, stderr, code) = rubash(
        "export LC_ALL=C\n\
         echo -e '\\u00e9' | od -An -tx1\n\
         echo -e '\\U0001f600' | od -An -tx1\n\
         LC_ALL=C.UTF-8\n\
         echo -e '\\u00e9' | od -An -tx1",
    );
    // C locale: literal "\u00E9" / "\U0001F600" bytes (hex upper-cased
    // by GNU's re-emission); UTF-8 locale: the raw c3 a9 of U+00E9.
    assert_eq!(
        stdout,
        " 5c 75 30 30 45 39 0a\n 5c 55 30 30 30 31 46 36 30 30 0a\n c3 a9 0a\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
