//! rubash#2759 — residual of #2706: a quoted prefix/suffix joined to an
//! UNQUOTED `${arr[@]}` splice still split inside the quoted part. The
//! scalar walkers carry the quoted-site verdict in-band
//! (QUOTED_EXPANSION_SITE), but the dollar_at segment assembly joined
//! first and split the joined string afterwards, erasing the
//! quoted/unquoted boundary. GNU subst.c:11229 expand_word_internal
//! tracks quoting per character through the single walk, and
//! subst.c:12273+ word splitting skips quoted separators.
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

#[test]
fn quoted_suffix_after_unquoted_array_at_is_protected() {
    // ${arr[@]}$"$b" and ${arr[@]}$"$b" plain-double-quote form: the quoted
    // tail joins the last element as ONE field ('w23 4'), never 'w23','4'.
    let (stdout, stderr, code) = rubash(
        "b='3 4'; arr=(w1 w2)\n\
         set -- ${arr[@]}$\"$b\"; printf '<%s>\\n' \"$@\"\n\
         set -- ${arr[@]}\"$b\"; printf '<%s>\\n' \"$@\"",
    );
    assert_eq!(stdout, "<w1>\n<w23 4>\n<w1>\n<w23 4>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_prefix_before_unquoted_array_at_is_protected() {
    // "$b"${arr[@]}: the quoted head keeps its space and joins the first
    // element ('3 4w1'); the split must not open inside the quoted head.
    let (stdout, stderr, code) = rubash(
        "b='3 4'; arr=(w1 w2)\n\
         set -- \"$b\"${arr[@]}; printf '<%s>\\n' \"$@\"",
    );
    assert_eq!(stdout, "<3 4w1>\n<w2>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn unquoted_then_quoted_suffix_after_array_at() {
    // ${arr[@]}$b"$a": $b splits (unquoted), the quoted "$a" tail stays
    // joined to the last split piece ('41 2').
    let (stdout, stderr, code) = rubash(
        "a='1 2'; b='3 4'; arr=(w1 w2)\n\
         set -- ${arr[@]}$b\"$a\"; printf '<%s>\\n' \"$@\"",
    );
    assert_eq!(stdout, "<w1>\n<w23>\n<41 2>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn unquoted_suffix_after_array_at_still_splits() {
    // Control: with no quoted part at all the junction splits through,
    // exactly as GNU does.
    let (stdout, stderr, code) = rubash(
        "b='3 4'; arr=(w1 w2)\n\
         set -- ${arr[@]}$b; printf '<%s>\\n' \"$@\"",
    );
    assert_eq!(stdout, "<w1>\n<w23>\n<4>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_junction_with_non_whitespace_ifs() {
    // IFS=: — the quoted tail's ':' characters are data, the unquoted
    // middle's are separators.
    let (stdout, stderr, code) = rubash(
        "IFS=:; a='1:2'; arr=(w1 w2)\n\
         set -- ${arr[@]}$a\"$a\"; printf '<%s>\\n' \"$@\"",
    );
    assert_eq!(stdout, "<w1>\n<w21>\n<21:2>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_suffix_after_unquoted_positional_at() {
    // $@ junction (not only ${arr[@]}): quoted tail joins the last
    // positional word whole.
    let (stdout, stderr, code) = rubash(
        "set -- 'x y' 'z w'\n\
         printf '<%s>\\n' $@$\"$2\"",
    );
    assert_eq!(stdout, "<x>\n<y>\n<z>\n<wz w>\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
