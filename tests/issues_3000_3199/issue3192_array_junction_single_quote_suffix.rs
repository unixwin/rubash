//! Regression pins for rubash#3192 — residual of #2759: a single-quoted
//! or ANSI-C-quoted segment in the SUFFIX position of an unquoted
//! `${arr[@]}` junction loses its trailing space. The #2759 segment
//! scanner gave double-quoted spans (and `$"..."`) a quoted literal
//! segment, so their IFS characters are quote-protected at the junction;
//! `'...'` and `$'...'` spans were only skipped over, stayed inside the
//! unquoted literal run, and the junction's field split / trailing-IFS
//! trim ate the quoted trailing space.
//!
//! GNU ownership: subst.c:11229 expand_word_internal tracks quoting
//! per character through the single walk; a single-quoted body is
//! quoted data handed whole to add_quoted_string (subst.c:11880 case
//! '\'' -> string_extract_single_quoted -> add_quoted_string), and a
//! `$'...'` body is decoded by ansicstr (subst.c:8766) into the same
//! quoted-data status. Neither span's characters ever field-split
//! (subst.c:12273+ word splitting skips quoted separators).
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for the
//! same scripts under LC_ALL=C.UTF-8 (oracle-verified).

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

fn check(script: &str, expected: &str) {
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stdout, expected);
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn single_quoted_suffix_space_survives_array_junction() {
    check(
        "arr=(w1 w2); set -- ${arr[@]}' '; printf '<%s>\\n' \"$@\"",
        "<w1>\n<w2 >\n",
    );
}

#[test]
fn single_quoted_suffix_text_and_space_survive_array_junction() {
    check(
        "arr=(w1 w2); set -- x${arr[@]}' y '; printf '<%s>\\n' \"$@\"",
        "<xw1>\n<w2 y >\n",
    );
}

#[test]
fn ansi_c_suffix_space_survives_array_junction() {
    check(
        "arr=(w1 w2); set -- ${arr[@]}$' '; printf '<%s>\\n' \"$@\"",
        "<w1>\n<w2 >\n",
    );
}

#[test]
fn ansi_c_suffix_text_and_space_survive_array_junction() {
    check(
        "arr=(w1 w2); set -- ${arr[@]}$'x '; printf '<%s>\\n' \"$@\"",
        "<w1>\n<w2x >\n",
    );
}

#[test]
fn double_quoted_suffix_space_control() {
    // Control: the double-quoted form already agreed with GNU (#2759).
    check(
        "arr=(w1 w2); set -- ${arr[@]}\" \"; printf '<%s>\\n' \"$@\"",
        "<w1>\n<w2 >\n",
    );
}

#[test]
fn single_quoted_prefix_control() {
    // Control: single-quoted PREFIX position already agreed with GNU.
    check(
        "arr=(w1 w2); set -- ' '${arr[@]}; printf '<%s>\\n' \"$@\"",
        "< w1>\n<w2>\n",
    );
}

#[test]
fn ansi_c_tab_suffix_control() {
    // Control: a $'\t' suffix already agreed with GNU.
    check(
        "arr=(w1 w2); set -- x${arr[@]}$'\\t'; printf '<%s>\\n' \"$@\"",
        "<xw1>\n<w2\t>\n",
    );
}

#[test]
fn single_quoted_suffix_dollar_stays_literal() {
    // The quoted body is data: `$x` inside '...' must NOT expand.
    check(
        "x=ZZ; arr=(w1 w2); set -- ${arr[@]}'$x'; printf '<%s>\\n' \"$@\"",
        "<w1>\n<w2$x>\n",
    );
}

#[test]
fn single_quoted_empty_suffix_control() {
    check(
        "arr=(w1 w2); set -- ${arr[@]}''; printf '<%s>\\n' \"$@\"",
        "<w1>\n<w2>\n",
    );
}
