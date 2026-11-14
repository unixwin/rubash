//! rubash#2819 — in a single-byte locale (C/POSIX), GNU splits on IFS
//! BYTES: membership in subst.c string_extract_verbatim falls to the
//! byte cmap (setifs builds ifs_cmap per byte) whenever the current
//! locale's MB_CUR_MAX is 1, so a multibyte IFS character separates on
//! EACH of its bytes. Only in a multibyte (UTF-8) locale does a complete
//! character match the IFS character list as a whole (wcschr). Rubash
//! split on whole characters in every locale.
//!
//! Expectations verified against GNU bash 5.3.20 (oracle runs).

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

fn rubash_env(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("LC_ALL", lc_all)
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
fn c_locale_read_splits_multibyte_ifs_by_byte() {
    // The filed repro: 0xC3 ends a, the immediately following 0xA9 is a
    // second separator (b empty), the remainder goes to the last var.
    let (stdout, stderr, code) = rubash(
        "LC_ALL=C\n\
         IFS='é' read -r a b c <<< 'xéyéz'\n\
         echo \"[$a][$b][$c]\"",
    );
    assert_eq!(stdout, "[x][][yéz]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn c_locale_read_without_r_splits_by_byte() {
    let (stdout, stderr, code) = rubash(
        "LC_ALL=C\n\
         IFS='é' read a b c <<< 'xéyéz'\n\
         echo \"[$a][$b][$c]\"",
    );
    assert_eq!(stdout, "[x][][yéz]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn c_locale_read_process_env_form() {
    // Same semantics when LC_ALL=C arrives via the process environment
    // instead of an in-script assignment.
    let (stdout, stderr, code) = rubash_env(
        "IFS='é' read -r a b c <<< 'xéyéz'\n\
         echo \"[$a][$b][$c]\"",
        "C",
    );
    assert_eq!(stdout, "[x][][yéz]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn c_locale_word_splitting_splits_multibyte_ifs_by_byte() {
    // Ordinary word splitting (expansion), not only read: both bytes of
    // 'é' are separators, so each produces its own field boundary.
    let (stdout, stderr, code) = rubash(
        "LC_ALL=C; IFS='é'; v='xéyéz'\n\
         set -- $v; printf '<%s>\\n' \"$@\"",
    );
    assert_eq!(stdout, "<x>\n<>\n<y>\n<>\n<z>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn c_locale_word_splitting_short_value() {
    let (stdout, stderr, code) = rubash(
        "LC_ALL=C; IFS='é'; v='aéb'\n\
         set -- $v; printf '<%s>\\n' \"$@\"",
    );
    assert_eq!(stdout, "<a>\n<>\n<b>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn c_locale_read_array_splits_by_byte() {
    // read -a (no -r): no remainder rule — every separator byte cuts.
    let (stdout, stderr, code) = rubash(
        "LC_ALL=C; IFS='é' read -a A <<< 'xéyéz'\n\
         printf '<%s>\\n' \"${A[@]}\"",
    );
    assert_eq!(stdout, "<x>\n<>\n<y>\n<>\n<z>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn c_locale_read_three_byte_ifs_char() {
    // '€' is three bytes (E2 82 AC): three separators in a row — two
    // empty fields between x and the remainder.
    let (stdout, stderr, code) = rubash(
        "LC_ALL=C; IFS='€' read -r a b c d <<< 'x€y€z'\n\
         echo \"[$a][$b][$c][$d]\"",
    );
    assert_eq!(stdout, "[x][][][y€z]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn c_locale_read_mixed_byte_and_whitespace_ifs() {
    // IFS=' é': the space stays IFS whitespace (collapses/trims), the
    // two bytes of 'é' are plain separators.
    let (stdout, stderr, code) = rubash(
        "LC_ALL=C; IFS=' é' read -r a b c <<< 'xé y z'\n\
         echo \"[$a][$b][$c]\"",
    );
    assert_eq!(stdout, "[x][][y z]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn utf8_locale_read_splits_multibyte_ifs_by_character() {
    // Control: in a UTF-8 locale the same IFS splits on the whole
    // character (GNU wcschr membership), both shells agree.
    let (stdout, stderr, code) = rubash(
        "LC_ALL=C.UTF-8; IFS='é' read -r a b c <<< 'xéyéz'\n\
         echo \"[$a][$b][$c]\"",
    );
    assert_eq!(stdout, "[x][y][z]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
