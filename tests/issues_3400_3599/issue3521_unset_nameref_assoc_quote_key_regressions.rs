//! Issue rubash#3521 (#3493 residual 1): `unset -v n["$k"]` where `n` is a
//! nameref to an associative array failed for keys that are a quote,
//! backslash or backtick character: rubash reported
//! `` `n[']': not a valid identifier `` while GNU removes the element.
//!
//! GNU oracle (bash 5.2.21 probes 2026-11-14, same version as the #3493
//! box probes): the operand word is W_ARRAYREF, so
//! `builtin_arrayref_flags` (builtins/common.c:1050-1051) gives it
//! VA_ONEWORD|VA_NOEXPAND; `tokenize_array_reference` (builtins/set.def
//! ~887, arrayfunc.c:1312-1313) then binds the subscript verbatim up to
//! the LAST `]` for an assoc array, and `find_variable` follows the
//! nameref chain to the referenced array, so `unbind_array_element`
//! (arrayfunc.c:1186-1187, VA_NOEXPAND: `akey = sub`) removes the key
//! verbatim. Rubash dropped those flags on its nameref detour:
//! `unset_through_nameref` re-parsed the operand with the quote-aware
//! `parse_array_subscript` and re-called `unset_array_element` with
//! `oneword=false`, so the quote-aware scan failed and the element
//! survived (the quote/backslash cases via the `not a valid identifier`
//! gate, the backtick silently via the lib fallback).
//!
//! Fix: thread the W_ARRAYREF flags through `unset_through_nameref` so
//! its subscripted-base arm splits verbatim and re-calls
//! `unset_array_element` with the same (arrayref, oneword) pair the
//! direct W_ARRAYREF call site uses. The plain-nameref-cell arm is
//! untouched: GNU applies `valid_array_reference(cell, 0)` there
//! (set.def:990-1010), and a quote/backtick-key cell cannot even be
//! created (`declare -n` rejects it as an invalid variable name for a
//! name reference), so no verbatim split is reachable on that arm.
//!
//! Guards below pin the faces GNU still rejects through a nameref:
//! indexed arrays, scalars, whole-word-quoted operands, empty keys.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.arg0("bash");
    }
    let output = command
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

/// The issue repro, verbatim: all three keys unset through the nameref,
/// dict ends empty.
#[test]
fn unset_nameref_assoc_quote_backslash_backtick_keys_via_loop() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict
dict['`']=2; dict["'"]=3; dict['\']=4
declare -n n=dict
for k in "${!dict[@]}"; do unset -v n["$k"]; done
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// Single-quote key via a double-quoted subscript through the nameref:
/// GNU removes it, rc 0.
#[test]
fn unset_nameref_assoc_single_quote_key_via_double_quoted_subscript() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict; dict["'"]=3; declare -n n=dict
k="'"
unset -v n["$k"]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// Backslash key via a double-quoted subscript through the nameref:
/// GNU removes it, rc 0.
#[test]
fn unset_nameref_assoc_backslash_key_via_double_quoted_subscript() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict; dict['\']=4; declare -n n=dict
k='\'
unset -v n["$k"]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// Backtick key via a double-quoted subscript through the nameref:
/// GNU removes it, rc 0.
#[test]
fn unset_nameref_assoc_backtick_key_via_double_quoted_subscript() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict; dict['`']=2; declare -n n=dict
k='`'
unset -v n["$k"]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// Single-quote key via unquoted expansion through the nameref: also a
/// W_ARRAYREF word, GNU removes it, rc 0.
#[test]
fn unset_nameref_assoc_single_quote_key_via_unquoted_expansion() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict; dict["'"]=3; declare -n n=dict
k="'"
unset -v n[$k]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// VA_ONEWORD takes the subscript up to the LAST `]` through the
/// nameref too: a `]` inside the key is data, only that element goes.
#[test]
fn unset_nameref_assoc_bracket_key_is_verbatim() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict; dict['a]b']=1; dict[x]=3; declare -n n=dict
k='a]b'; unset -v n["$k"]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=([x]=\"3\" )\n");
}

/// Without `-v` the nameref route removes the element as well (GNU
/// probe A6), and the nameref itself survives.
#[test]
fn unset_nameref_assoc_single_quote_key_without_v_flag() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict; dict["'"]=3; declare -n n=dict
k="'"
unset n["$k"]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// A nameref chain (n1 -> n2 -> dict) resolves to the referenced array
/// (GNU probe A7): the element goes, both namerefs survive and still
/// point at the dict.
#[test]
fn unset_nameref_chain_assoc_single_quote_key() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict; dict["'"]=3; declare -n n2=dict; declare -n n1=n2
k="'"
unset -v n1["$k"]
n1[x]=9
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=([x]=\"9\" )\n");
}

/// Control: a plain key through a nameref chain resolves through the
/// whole chain (GNU find_variable follows it): the element goes, both
/// namerefs survive.
#[test]
fn unset_nameref_chain_assoc_plain_key() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict; dict[x]=3; declare -n n2=dict; declare -n n1=n2
unset -v n1[x]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// Guard: the same spelling on an INDEXED array through a nameref stays
/// rejected exactly as GNU rejects it.
#[test]
fn unset_nameref_indexed_single_quote_key_still_invalid() {
    let (stdout, stderr, _) = rubash(
        r#"declare -a arr; arr=(a b c); declare -n n=arr
k="'"
unset -v n["$k"]; echo "rc=$?"
declare -p arr"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `n[']': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(
        stdout,
        "rc=1\ndeclare -a arr=([0]=\"a\" [1]=\"b\" [2]=\"c\")\n"
    );
}

/// Guard: the same spelling on a scalar through a nameref stays
/// rejected, as GNU does.
#[test]
fn unset_nameref_scalar_single_quote_key_still_invalid() {
    let (stdout, stderr, _) = rubash(
        r#"v=hello; declare -n n=v
k="'"
unset -v n["$k"]; echo "rc=$?"
declare -p v"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `n[']': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(stdout, "rc=1\ndeclare -- v=\"hello\"\n");
}

/// Guard: a whole-word double-quoted operand through a nameref is NOT a
/// W_ARRAYREF word, so GNU rejects it too.
#[test]
fn unset_nameref_assoc_quote_key_whole_word_quoted_still_invalid() {
    let (stdout, stderr, _) = rubash(
        r#"declare -A dict; dict["'"]=3; declare -n n=dict
k="'"
unset -v "n[$k]"; echo "rc=$?"
declare -p dict"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `n[']': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(stdout, "rc=1\ndeclare -A dict=([\"'\"]=\"3\" )\n");
}

/// Guard: an empty key through a nameref stays rejected, as GNU does.
#[test]
fn unset_nameref_assoc_empty_key_still_invalid() {
    let (stdout, stderr, _) = rubash(
        r#"declare -A dict; dict[x]=2; declare -n n=dict
k=""
unset -v n["$k"]; echo "rc=$?"
declare -p dict"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `n[]': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(stdout, "rc=1\ndeclare -A dict=([x]=\"2\" )\n");
}
