//! rubash#1621 — under a UTF-8 locale, GNU resolves POSIX character
//! classes through wctype/iswctype (glibc's Unicode-derived tables),
//! so multibyte characters match: É is [[:upper:]], é is
//! [[:lower:]]/[[:alpha:]]. rubash's pattern matcher used an
//! ASCII-only class table on every surface sharing it ([[ ]], case,
//! ${var#pat} removal, =~), so all of them missed. Under LC_ALL=C both
//! shells agree on the byte semantics (pinned as the control group;
//! see also #1533).
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! LC_ALL=C.UTF-8 and LC_ALL=C runs), including the measured glibc
//! edge rules: non-ASCII Nd digits are `alpha` but not `digit`,
//! U+00A0 is not `space`, U+3000 is `space`/`blank` but not `graph`.

use std::process::Command;

fn rubash(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", lc_all)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn utf8_locale_posix_classes_match_multibyte_characters() {
    let script = "\
        [[ \u{c9} == [[:upper:]] ]] && echo upper-hit\n\
        [[ \u{e9} == [[:lower:]] ]] && echo lower-hit\n\
        [[ \u{e9} == [[:alpha:]] ]] && echo alpha-hit\n\
        case \u{c9} in [[:upper:]]) echo case-hit;; esac\n\
        z=\u{c9}abc; echo \"rem:${z#[[:upper:]]}\"\n\
        [[ \u{c9} =~ ^[[:upper:]]$ ]] && echo re-hit\n\
        [[ \u{665} == [[:alpha:]] ]] && echo nd-alpha-hit\n\
        [[ \u{665} == [[:digit:]] ]] || echo digit-miss-ok\n\
        [[ \u{665} =~ ^[[:alpha:]]$ ]] && echo re-nd-alpha-hit\n\
        [[ \u{e9} == [[:punct:]] ]] || echo e-not-punct-ok\n\
        [[ \u{c9} == [^[:lower:]] ]] && echo neg-hit\n\
        [[ 5 == [[:upper:][:digit:]] ]] && echo combo-hit\n\
        nbsp=$(printf '\\xc2\\xa0'); [[ $nbsp == [[:space:]] ]] || echo nbsp-not-space-ok\n\
        ideo=$(printf '\\xe3\\x80\\x80'); [[ $ideo == [[:space:]] ]] && echo ideo-space-hit\n\
        [[ $ideo == [[:graph:]] ]] || echo ideo-not-graph-ok\n";
    let (stdout, stderr, code) = rubash(script, "C.UTF-8");
    assert_eq!(
        stdout,
        "upper-hit\nlower-hit\nalpha-hit\ncase-hit\nrem:abc\nre-hit\n\
         nd-alpha-hit\ndigit-miss-ok\nre-nd-alpha-hit\ne-not-punct-ok\n\
         neg-hit\ncombo-hit\nnbsp-not-space-ok\nideo-space-hit\nideo-not-graph-ok\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn c_locale_posix_classes_stay_byte_semantics() {
    let script = "\
        [[ \u{c9} == [[:upper:]] ]] || echo c-upper-miss\n\
        [[ \u{e9} == [[:alpha:]] ]] || echo c-alpha-miss\n\
        z=\u{c9}abc; echo \"rem:${z#[[:upper:]]}\"\n\
        [[ \u{c9} =~ ^[[:upper:]]$ ]] || echo c-re-miss\n";
    let (stdout, stderr, code) = rubash(script, "C");
    assert_eq!(
        stdout,
        "c-upper-miss\nc-alpha-miss\nrem:\u{c9}abc\nc-re-miss\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
