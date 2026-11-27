//! rubash#876 regressions: `mark_quoted_ifs_from_raw` applies its quoted-IFS
//! marks on the same byte-offset basis `push_quoted_char` records.
//!
//! `push_quoted_char` recorded `simulated.len()` — a BYTE offset — while the
//! apply loop counted one per `char`. A multi-byte char before a quoted IFS
//! char shifted every later record past the apply loop's counter, so the
//! quoted blank took no glue and the field splitter released it: with
//! IFS=' :', `"é a"$w` split into `<é><aW>` where GNU keeps `<é aW>` (the
//! apply loop now advances by `ch.len_utf8()`, so the byte offsets line up
//! again). Quoted non-whitespace IFS chars were backstopped by
//! `mark_literal_ifs_chars`, which is why only the whitespace face diverged.
//!
//! This is the #868/#874 third defect in the same function (PR #874 CR):
//! the #874 carrier awareness lets more multi-byte words through the
//! simulation gate, widening the reach of the miscount. The carrier arms and
//! the double-mark protection land above this fix and stay intact — every
//! expectation below is byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file batteries 2026-10-19, artifacts under
//! the wt876 lane).

use std::process::Command;

/// Run a script FILE in its own scratch directory and return
/// (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-ifsmark876-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_script(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, expected_stdout, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}

/// The #876 ticket face: a multi-byte char before a quoted IFS blank must
/// not shift the blank's mark out from under the apply loop. 2-byte (é, ä),
/// 3-byte (中文) and 4-byte (emoji) leads, plus the decoded-tab span, with
/// the unquoted `$w` tail still splitting.
#[test]
fn multibyte_before_quoted_blank_keeps_protection() {
    assert_script(
        concat!(
            "w=W\n",
            "IFS=' :'\n",
            "printf '<%s>' \"é a\"$w; echo\n",
            "printf '<%s>' \"中文 a\"$w; echo\n",
            "printf '<%s>' \"é\"$'\\t'\"a\"$w; echo\n",
            "printf '<%s>' \"ä ö\"$w; echo\n",
            "printf '<%s>' \"\u{1f600} x\"$w; echo\n",
        ),
        "<é aW>\n<中文 aW>\n<é\taW>\n<ä öW>\n<\u{1f600} xW>\n",
    );
}

/// Non-whitespace quoted IFS chars behind a multi-byte lead stay protected
/// (the literal-mark backstop used to hide this face) and a quoted blank
/// that mark_literal_ifs_chars already glued is not double-marked — the
/// #868 guard holds on the byte-offset basis too.
#[test]
fn multibyte_before_quoted_colon_and_double_mark_face() {
    assert_script(
        concat!(
            "w=W\n",
            "IFS=:\n",
            "printf '<%s>' \"é:a\"$w; echo\n",
            "printf '<%s>' \"中文:a\"$w; echo\n",
            "IFS=' :'\n",
            "printf '<%s>' \"é: a\"$w; echo\n",
            "printf '<%s>' \"中文 :a\"$w; echo\n",
        ),
        "<é:aW>\n<中文:aW>\n<é: aW>\n<中文 :aW>\n",
    );
}

/// Single-quote spans, escaped blanks and nested quoted segments with a
/// multi-byte lead — all quoted data, all protected, `$w` still splits.
#[test]
fn single_quote_escape_and_nested_spans_behind_multibyte() {
    assert_script(
        concat!(
            "w=W\n",
            "IFS=' :'\n",
            "printf '<%s>' 'é a'$w; echo\n",
            "printf '<%s>' é\\ a$w; echo\n",
            "printf '<%s>' \"中\"'文 a'$w; echo\n",
        ),
        "<é aW>\n<é aW>\n<中文 aW>\n",
    );
}

/// ANSI-C decoded multi-byte leads (`$'\u00e9'`, `$'\u4e2d'`): the decode
/// output carries guard/quoting bytes the simulation must match, and the
/// quoted blank after the decoded char must still glue.
#[test]
fn ansi_c_decoded_multibyte_leads() {
    assert_script(
        concat!(
            "w=W\n",
            "IFS=' :'\n",
            "printf '<%s>' $'\\u00e9 a'$w; echo\n",
            "printf '<%s>' \"x\"$'\\u4e2d'\" a\"$w; echo\n",
        ),
        "<é aW>\n<x中 aW>\n",
    );
}

/// Carrier tails: the unquoted expansion result itself carries multi-byte
/// chars and IFS separators — the quoted segment stays protected while the
/// tail splits exactly like GNU.
#[test]
fn multibyte_carrier_tails_still_split() {
    assert_script(
        concat!(
            "w='中:x'\n",
            "IFS=:\n",
            "printf '<%s>' \"é:q\"$w; echo\n",
            "w='中 x'\n",
            "IFS=' :'\n",
            "printf '<%s>' \"é q\"$w; echo\n",
        ),
        "<é:q中><x>\n<é q中><x>\n",
    );
}

/// Unset/empty expansions behind a multi-byte quoted lead invent no fields,
/// and a fully-quoted multi-byte word stays one field.
#[test]
fn unset_expansion_and_quoted_whole_word_behind_multibyte() {
    assert_script(
        concat!(
            "IFS=' :'\n",
            "unset w\n",
            "printf '<%s>' \"é a\"$w; echo\n",
            "printf '<%s>' \"é a\"; echo\n",
        ),
        "<é a>\n<é a>\n",
    );
}

/// A multi-byte char AFTER the quote close is unquoted data (splits like
/// GNU), and the plain-ASCII control faces keep their master behavior.
#[test]
fn multibyte_after_quote_close_and_ascii_controls() {
    assert_script(
        concat!(
            "w=W\n",
            "IFS=' :'\n",
            "printf '<%s>' \"é a\"中$w; echo\n",
            "printf '<%s>' \"a b\"é$w; echo\n",
            "printf '<%s>' \"a b\"$w; echo\n",
            "printf '<%s>' \"a:b\"$w; echo\n",
            "IFS=:\n",
            "printf '<%s>' \"a:b\"$w; echo\n",
            "printf '<%s>' a\\:b$w; echo\n",
        ),
        "<é a中W>\n<a béW>\n<a bW>\n<a:bW>\n<a:bW>\n<a:bW>\n",
    );
}
