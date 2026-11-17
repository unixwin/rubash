//! rubash#3787 — `echo -e` silently dropped `\u`/`\U` escapes whose code
//! point is not a valid Unicode scalar (surrogates U+D800..U+DFFF, or
//! above U+10FFFF) under a UTF-8 locale. GNU encodes the numeric value
//! as-is through u32cconv -> u32toutf8 (lib/sh/unicode.c:239, :154):
//! surrogates as 3-byte CESU-8, U+110000 as the 4-byte form, values up
//! to < 0x80000000 as 5/6-byte forms, and values >= 0x80000000 as
//! nothing (in every locale). In the C locale GNU keeps the escape
//! literal (u32tocesc fallback) except for the >= 0x80000000 band,
//! which is dropped there too.
//!
//! Expectations verified against GNU bash 5.3.20 and system bash 5.2.21
//! (script files, od hex bytes); the two agree on every probe below.
//! `$'...'` and `printf %b` already match GNU here and are out of scope
//! (controls only).

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
fn echo_e_encodes_nonscalar_unicode_escapes_like_gnu() {
    let (stdout, stderr, code) = rubash(
        "LC_ALL=C.utf8\n\
         echo -e 'A\\ud83d\\ude00B' | od -An -tx1\n\
         echo -e 'A\\ud800B' | od -An -tx1\n\
         echo -e 'A\\udfffB' | od -An -tx1\n\
         echo -e 'A\\ud800\\udc00B' | od -An -tx1\n\
         echo -e 'A\\udbffB' | od -An -tx1\n\
         echo -e 'A\\udc00B' | od -An -tx1\n\
         echo -e 'A\\U0000D800B' | od -An -tx1\n\
         echo -e 'A\\U00110000B' | od -An -tx1\n\
         echo -e 'A\\U00200000B' | od -An -tx1\n\
         echo -e 'A\\U80000000B' | od -An -tx1\n\
         echo -e 'A\\UFFFFFFFFB' | od -An -tx1\n\
         echo -e 'A\\u00e9B' | od -An -tx1\n\
         echo -e 'A\\U0001F600B' | od -An -tx1\n\
         echo -e 'A\\ufffeB' | od -An -tx1\n\
         echo -e 'A\\U0010FFFFB' | od -An -tx1\n\
         LC_ALL=C\n\
         echo -e 'A\\ud83d\\ude00B' | od -An -tx1\n\
         echo -e 'A\\U00110000B' | od -An -tx1\n\
         echo -e 'A\\U80000000B' | od -An -tx1\n\
         LC_ALL=C.utf8\n\
         printf '%b' 'A\\ud83d\\ude00B' | od -An -tx1",
    );
    // One od line per probe, in script order. UTF-8 locale: CESU-8
    // surrogates, 4-byte U+110000, 5-byte U+200000, >= 0x80000000
    // dropped; valid scalars (incl. noncharacter U+FFFE, max U+10FFFF)
    // unchanged. C locale: escapes stay literal, except the dropped
    // >= 0x80000000 band. Last line is the printf %b control (no
    // trailing newline from printf itself).
    let expected = [
        " 41 ed a0 bd ed b8 80 42 0a",
        " 41 ed a0 80 42 0a",
        " 41 ed bf bf 42 0a",
        " 41 ed a0 80 ed b0 80 42 0a",
        " 41 ed af bf 42 0a",
        " 41 ed b0 80 42 0a",
        " 41 ed a0 80 42 0a",
        " 41 f4 90 80 80 42 0a",
        " 41 f8 88 80 80 80 42 0a",
        " 41 42 0a",
        " 41 42 0a",
        " 41 c3 a9 42 0a",
        " 41 f0 9f 98 80 42 0a",
        " 41 ef bf be 42 0a",
        " 41 f4 8f bf bf 42 0a",
        " 41 5c 75 44 38 33 44 5c 75 44 45 30 30 42 0a",
        " 41 5c 55 30 30 31 31 30 30 30 30 42 0a",
        " 41 42 0a",
        " 41 ed a0 bd ed b8 80 42",
    ]
    .join("\n")
        + "\n";
    assert_eq!(stdout, expected);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
