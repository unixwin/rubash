//! rubash#2596 — a `:?` nested inside a `:-`/`:+`/`:=` operator
//! word is misjudged by the command-level error pre-scan even when
//! the inner element is SET and the subscript is a literal:
//! `b=(p q); echo ${u:-${b[0]:?err}}` dies in rubash with the outer
//! operator's text mangled into the diagnostic
//! (`u:-${b[0]: err}`), where GNU prints `p`.
//!
//! GNU law: parameter_brace_expand (subst.c:9777) adjudicates ONE
//! expansion per call. The outer name is extracted first
//! (string_extract SX_VARNAME, subst.c:9805-9810 — it stops at the
//! outer operator), the outer value is fetched once, and the
//! operator word is cut out brace-aware
//! (extract_dollar_brace_string, subst.c:10150; definition
//! subst.c:1825 — nested `${...}` units are skipped whole). The
//! word is expanded only when the operator selects it
//! (parameter_brace_expand_rhs, subst.c:7966), and a nested
//! expansion inside it is adjudicated by its own recursive
//! parameter_brace_expand call, under its own name. The rubash
//! pre-scan instead runs `parse_parameter_error_operator`
//! (src/executor/parameter_ops.rs) over the OUTER span's inner text
//! with a naive first-occurrence `split_once(":?")`: for
//! `u:-${b[0]:?err}` it finds the INNER `:?`, takes
//! `u:-${b[0]` as the parameter name (accepted as an array
//! reference by is_parameter_error_name), fetches it (unset), and
//! reports the mangled name — before the scan's own
//! nested-operator-word arm (which splits subscript-aware) is ever
//! consulted.
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
fn issue2596_nested_in_default_word_inner_set() {
    let (stdout, stderr, code) = rubash("b=(p q); echo \"out=${u:-${b[0]:?err}}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "out=p\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2596_nested_in_alternate_word_inner_set() {
    let (stdout, stderr, code) = rubash("b=(p q); u=x; echo \"out=${u:+${b[0]:?err}}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "out=p\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2596_nested_in_assign_word_inner_set() {
    // The := word is selected (u unset), the inner fragment yields
    // p, and the assignment lands: u=p afterwards (chain state).
    let (stdout, stderr, code) = rubash("b=(p q); echo \"out=${u:=${b[0]:?err}} u=$u\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "out=p u=p\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2596_nested_in_alternate_word_outer_unset_not_evaluated() {
    // u2 is unset, so :+ never selects its word: the inner :? must
    // not fire at either layer.
    let (stdout, stderr, code) = rubash("echo \"out=[${u2:+${w:?innererr}}]\"; echo alive4");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "out=[]\nalive4\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2596_nested_scalar_in_default_word() {
    let (stdout, stderr, code) = rubash("s=hi; echo \"out=${u3:-${s:?err}}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "out=hi\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2596_nested_in_default_word_outer_set_not_evaluated() {
    let (stdout, stderr, code) = rubash("u=ok; echo \"out=${u:-${w:?innererr}}\"; echo alive6");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "out=ok\nalive6\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2596_double_nested_default_word() {
    let (stdout, stderr, code) = rubash("b=(p q); echo \"out=${u:-${v:-${b[0]:?err}}}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "out=p\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2596_nested_error_names_inner_scalar() {
    // The word IS selected (u unset) and the inner parameter is
    // genuinely unset: GNU reports the INNER name and aborts
    // (subst.c:8216 parameter_brace_expand_error on the recursive
    // call). The diagnostic must not carry the outer text.
    let (stdout, stderr, code) = rubash("echo ${u:-${w:?innererr}}; echo alive3");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "", "the command must abort, got: {stdout}");
    assert!(stderr.contains("w: innererr"), "stderr: {stderr}");
    assert!(
        !stderr.contains("u:-"),
        "outer operator text must not leak into the diagnostic: {stderr}"
    );
}

#[test]
fn issue2596_nested_error_names_inner_element() {
    let (stdout, stderr, code) = rubash("b=(p); echo ${u:-${b[3]:?err}}; echo alive9");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "", "the command must abort, got: {stdout}");
    assert!(stderr.contains("b[3]: err"), "stderr: {stderr}");
    assert!(
        !stderr.contains("u:-"),
        "outer operator text must not leak into the diagnostic: {stderr}"
    );
}

#[test]
fn issue2596_nested_in_error_message_inner_unset() {
    // The :? message word is expanded by
    // parameter_brace_expand_error (subst.c:8216-8226) before the
    // outer report; a :? inside it pre-empts and names the inner
    // variable (the rubash#2343 rule, kept as a control here).
    let (stdout, stderr, code) = rubash("echo ${u:?${w:?inner}}; echo alive7");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "", "the command must abort, got: {stdout}");
    assert!(stderr.contains("w: inner"), "stderr: {stderr}");
}

#[test]
fn issue2596_nested_in_error_message_inner_set() {
    // The message word expands cleanly to `deep`, which becomes the
    // outer parameter's diagnostic text.
    let (stdout, stderr, code) = rubash("w=deep; echo ${u:?${w:?inner}}; echo alive8");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "", "the command must abort, got: {stdout}");
    assert!(stderr.contains("u: deep"), "stderr: {stderr}");
}
