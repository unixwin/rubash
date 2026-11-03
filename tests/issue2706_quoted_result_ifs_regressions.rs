//! rubash#2706 — in a mixed word, an expansion result produced inside
//! a quoted segment lost its IFS protection: `$a"$b"` split inside the
//! quoted `$b` result, while a quoted LITERAL segment on the same word
//! was protected. GNU subst.c:11229 expand_word_internal tracks
//! quoting per character through the single walk (a quoted expansion
//! result is added via add_quoted_string, subst.c:11862) and
//! word_list_split splits only on unprotected IFS characters; the
//! subst.c:11552 comment states the contract: the result "will be
//! split, but this portion should not be".
//!
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
fn quoted_parameter_result_is_not_split() {
    let (stdout, stderr, code) = rubash(
        "a='1 2'; b='3 4'\n\
         printf '<%s>\\n' $a\"$b\"\n\
         printf '<%s>\\n' \"$a\"$b\n\
         printf '<%s>\\n' \"$b\"$a",
    );
    assert_eq!(stdout, "<1>\n<23 4>\n<1 23>\n<4>\n<3 41>\n<2>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_substitution_result_is_not_split() {
    let (stdout, stderr, code) = rubash(
        "a='1 2'\n\
         printf '<%s>\\n' $a\"$(echo 'p q')\"\n\
         printf '<%s>\\n' $a\"`echo 'p q'`\"",
    );
    assert_eq!(stdout, "<1>\n<2p q>\n<1>\n<2p q>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_result_protection_in_for_list() {
    let (stdout, stderr, code) = rubash(
        "a='1 2'; b='3 4'\n\
         for x in $a\"$b\"; do echo \"[$x]\"; done",
    );
    assert_eq!(stdout, "[1]\n[23 4]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// The quoted neighbor of an UNQUOTED `${arr[@]}` splice: GNU joins
/// the quoted segment onto the first/last element as a quoted string
/// before splitting, so only the array's own characters split. The
/// dollar_at segment assembly (quoted_positional_at_segments /
/// expand_quoted_positional_at_segments) joins first and splits the
/// joined string, erasing that boundary — it needs the same site
/// verdict plumbed through its segment model, a separate surface
/// from the scalar walks fixed here.
#[test]
#[ignore = "issue#2706 residual: quoted prefix/suffix joined to an unquoted ${arr[@]} splice still splits inside the quoted part (${arr[@]}$\"$b\" -> w3, 4 instead of 'w3 4'); the dollar_at segment assembly does not yet carry the quoted-site protection"]
fn quoted_result_protection_at_array_junction() {
    let (stdout, stderr, code) = rubash(
        "a='1 2'; b='3 4'; arr=('x y' 'z w')\n\
         printf '<%s>\\n' ${arr[@]}$\"$b\"\n\
         printf '<%s>\\n' \"$b\"${arr[@]}\n\
         printf '<%s>\\n' ${arr[@]}$b\"$a\"",
    );
    assert_eq!(
        stdout,
        "<x>\n<y>\n<z>\n<w3 4>\n\
         <3 4x>\n<y>\n<z>\n<w>\n\
         <x>\n<y>\n<z>\n<w3>\n<41 2>\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_result_protection_with_non_whitespace_ifs() {
    let (stdout, stderr, code) = rubash(
        "IFS=:\n\
         a='1:2'; b='3:4'\n\
         printf '<%s>\\n' $a\"$b\"\n\
         printf '<%s>\\n' \"$a\"$b",
    );
    assert_eq!(stdout, "<1>\n<23:4>\n<1:23>\n<4>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn unquoted_junctions_still_split() {
    // Control faces that already agreed: both-unquoted junctions merge
    // and split through, and a quoted literal segment protects only
    // itself.
    let (stdout, stderr, code) = rubash(
        "a='1 2'; b='3 4'\n\
         printf '<%s>\\n' $a$b\n\
         printf '<%s>\\n' $a\"X Y\"$b",
    );
    assert_eq!(stdout, "<1>\n<23>\n<4>\n<1>\n<2X Y3>\n<4>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
