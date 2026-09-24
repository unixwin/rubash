//! UTF-8 boundary-hygiene regressions for the sites surfaced by
//! scripts/check-utf8-boundary-hygiene.sh.
//!
//! The guard documents the failure class behind unixwin/niubash#92:
//! widening a payload byte with `byte as char` Latin-1-encodes multi-byte
//! text (a UTF-8 lead+continuation run becomes one mojibake char per
//! byte), and slicing a `&str` at a character counter panics on non-ASCII
//! input. These pin the executor paths that still did the former.

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

/// regex_rhs_marked_chars widened every UTF-8 byte of the quoted/expanded
/// `[[ =~ ]]` RHS into a Latin-1 char, so a quoted non-ASCII pattern
/// reached the regex engine mojibaked and the match silently failed while
/// the identical unquoted form matched. GNU cond_expand_word quotes the
/// expanded characters, not a re-encoding of their bytes.
#[test]
fn cond_regex_quoted_non_ascii_rhs_still_matches() {
    let (stdout, stderr, code) =
        rubash("s=中文; pattern='文'; [[ $s =~ \"$pattern\" ]]; echo $? ${BASH_REMATCH[0]}");
    assert_eq!(stdout, "0 文\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// Control for the same operator: the unquoted non-ASCII RHS must keep
/// matching, so the quoted-arm fix cannot break the working path.
#[test]
fn cond_regex_unquoted_non_ascii_rhs_still_matches() {
    let (stdout, stderr, code) = rubash("s=中文; pattern='文'; [[ $s =~ $pattern ]]; echo $?");
    assert_eq!(stdout, "0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// compound_element_xtrace_text widened the compound-assignment element
/// bytes, so `set -x` printed `arr=(ä¸­æ–‡)` for `arr=(中文)`.
#[test]
fn xtrace_compound_assignment_keeps_multibyte_element() {
    let (stdout, stderr, code) = rubash("set -x; arr=(中文) true");
    assert!(
        stderr.contains("arr=(中文)"),
        "stderr: {stderr}, stdout: {stdout}"
    );
    assert_eq!(code, Some(0));
}

/// The `$'...'` element rewrite must decode real characters: the body was
/// accumulated through the same widening, so `decode_ansi_c_quoted` saw
/// mojibake before it could decode anything.
#[test]
fn xtrace_compound_assignment_ansi_c_element_keeps_multibyte() {
    let (stdout, stderr, code) = rubash("set -x; arr=($'中') true");
    assert!(stderr.contains("中"), "stderr: {stderr}, stdout: {stdout}");
    assert_eq!(code, Some(0));
}
