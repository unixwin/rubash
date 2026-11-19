//! Issue rubash#2657: `unset -v dict["$k"]` of an associative element whose
//! key is a quote, backslash or backtick character failed: the quote and
//! backslash keys were rejected as "not a valid identifier", and the
//! backtick key was silently kept. The expanded word's quoting bytes are
//! literal key data, not syntax, but the quote-aware subscript scan
//! (parse_array_subscript) misread them, so the element never unbound.
//!
//! GNU oracle (issue text: bash 5.3.20; box probes 2026-11-13: bash 5.2.21):
//! the W_ARRAYREF form binds the subscript verbatim up to the last `]`
//! (VA_ONEWORD), so all three keys unset with rc 0 and the dict ends
//! empty. The same spellings stay rejected for indexed arrays and scalars,
//! for a whole-word double-quoted operand, and for an empty key - those
//! faces are pinned below as guards.
//!
//! Fix: in `unset_array_element`, a W_ARRAYREF (oneword) operand of an
//! associative array splits at the first `[` and the last `]` with no
//! quote/escape/backquote scan.

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

/// The issue repro, verbatim: all three keys unset, dict ends empty.
#[test]
fn unset_assoc_quote_backslash_backtick_keys_via_loop() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict
dict['`']=2; dict["'"]=3; dict['\']=4
for k in "${!dict[@]}"; do unset -v dict["$k"]; done
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// Single-quote key via a double-quoted subscript: GNU removes it, rc 0.
#[test]
fn unset_assoc_single_quote_key_via_double_quoted_subscript() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict
dict["'"]=3
k="'"
unset -v dict["$k"]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// Backslash key via a double-quoted subscript: GNU removes it, rc 0.
#[test]
fn unset_assoc_backslash_key_via_double_quoted_subscript() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict
dict['\']=4
k='\'
unset -v dict["$k"]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// Backtick key via a double-quoted subscript: GNU removes it, rc 0.
/// (Pre-fix rubash kept it SILENTLY - no diagnostic, no removal.)
#[test]
fn unset_assoc_backtick_key_via_double_quoted_subscript() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict
dict['`']=2
k='`'
unset -v dict["$k"]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// Single-quote key via unquoted expansion: also a W_ARRAYREF word, GNU
/// removes it, rc 0.
#[test]
fn unset_assoc_single_quote_key_via_unquoted_expansion() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict
dict["'"]=3
k="'"
unset -v dict[$k]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=()\n");
}

/// VA_ONEWORD takes the subscript up to the LAST `]`: keys containing
/// `]` or `[` are literal data, and only those elements go.
#[test]
fn unset_assoc_bracket_keys_are_verbatim() {
    let (stdout, stderr, rc) = rubash(
        r#"declare -A dict
dict['a]b']=1; dict['a[b']=2; dict[x]=3
k='a]b'; unset -v dict["$k"]
k='a[b'; unset -v dict["$k"]
declare -p dict"#,
    );
    assert_eq!(stderr, "", "stderr not empty: {stderr}");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "declare -A dict=([x]=\"3\" )\n");
}

/// Guard: the same spelling on an INDEXED array stays rejected exactly
/// as GNU rejects it - the verbatim split is assoc-only.
#[test]
fn unset_indexed_single_quote_key_still_invalid() {
    let (stdout, stderr, _) = rubash(
        r#"declare -a arr; arr=(a b c)
k="'"
unset -v arr["$k"]; echo "rc=$?"
declare -p arr"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `arr[']': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(
        stdout,
        "rc=1\ndeclare -a arr=([0]=\"a\" [1]=\"b\" [2]=\"c\")\n"
    );
}

/// Guard: the same spelling on a scalar stays rejected, as GNU does.
#[test]
fn unset_scalar_single_quote_key_still_invalid() {
    let (stdout, stderr, _) = rubash(
        r#"v=hello
k="'"
unset -v v["$k"]; echo "rc=$?"
declare -p v"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `v[']': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(stdout, "rc=1\ndeclare -- v=\"hello\"\n");
}

/// Guard: a whole-word double-quoted operand is NOT a W_ARRAYREF word,
/// so GNU rejects it too - the gate must keep rejecting it.
#[test]
fn unset_assoc_quote_key_whole_word_quoted_still_invalid() {
    let (stdout, stderr, _) = rubash(
        r#"declare -A dict; dict["'"]=3
k="'"
unset -v "dict[$k]"; echo "rc=$?"
declare -p dict"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `dict[']': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(stdout, "rc=1\ndeclare -A dict=([\"'\"]=\"3\" )\n");
}

/// Guard: an empty key stays rejected, as GNU rejects it.
#[test]
fn unset_assoc_empty_key_still_invalid() {
    let (stdout, stderr, _) = rubash(
        r#"declare -A dict; dict[x]=2
k=""
unset -v dict["$k"]; echo "rc=$?"
declare -p dict"#,
    );
    assert_eq!(
        stderr, "bash: line 3: unset: `dict[]': not a valid identifier\n",
        "wrong stderr"
    );
    assert_eq!(stdout, "rc=1\ndeclare -A dict=([x]=\"2\" )\n");
}
