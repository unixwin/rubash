//! rubash#2657 — `unset -v` of an associative element whose key is a
//! quote/backslash-class character must remove the element. GNU
//! tokenizes the operand into base name + subscript BEFORE any
//! identifier check and validates the base name only:
//! builtins/set.def:891 tokenize_array_reference runs first
//! (arrayfunc.c:1288), valid_identifier is applied to the truncated
//! base name (set.def, right after the tokenize call), and the
//! element is removed by unbind_array_element (arrayfunc.c:1141).
//! For a W_ARRAYREF word (`dict["$k"]` — `[` outside quotes in the
//! raw token) builtin_arrayref_flags (builtins/common.c:1040) yields
//! VA_ONEWORD|VA_NOEXPAND, so the expanded subscript text is taken
//! verbatim up to the final `]` — a key of `'`, `\` or a backtick is
//! an ordinary key.
//!
//! The defect: rubash re-validated the EXPANDED operand with a
//! quote-aware whole-string scan, so `dict[']` read as an
//! unterminated quote and `dict[\]` as an escaped bracket, and the
//! elements survived with a bogus "not a valid identifier".
//!
//! Expectations measured byte-for-byte against GNU bash 5.3.20 with
//! the same -c harness shape (2026-11-09). Cells whose GNU stderr is
//! non-empty assert stdout/rc byte-exact plus stderr substrings,
//! because diagnostic lines carry a shell-specific program prefix.

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_c(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// Clean cell: exact stdout, empty stderr, exit 0.
fn assert_clean(script: &str, expected_stdout: &str) {
    let run = run_c(script);
    assert_eq!(run.stdout, expected_stdout, "script: {script}");
    assert_eq!(run.stderr, "", "script: {script}");
    assert_eq!(run.code, Some(0), "script: {script}");
}

/// Diagnostic cell: exact stdout and exit code; stderr must contain
/// each required substring exactly `count` times and never the panic
/// signature.
fn assert_diag(
    script: &str,
    expected_stdout: &str,
    expected_code: i32,
    required: &[(&str, usize)],
) {
    let run = run_c(script);
    assert_eq!(run.stdout, expected_stdout, "script: {script}");
    assert_eq!(run.code, Some(expected_code), "script: {script}");
    for (needle, count) in required {
        assert_eq!(
            run.stderr.matches(needle).count(),
            *count,
            "stderr substring {needle:?} count, script: {script}, stderr: {:?}",
            run.stderr
        );
    }
    assert!(
        !run.stderr.contains("panicked"),
        "script: {script}, stderr: {:?}",
        run.stderr
    );
}

/// The issue's headline shape (assoc9.sub del chain): keys backtick,
/// single quote and backslash, removed one by one through the
/// expanded `dict["$k"]` operand form.
#[test]
fn issue2657_unset_expanded_form_quote_and_backslash_keys() {
    assert_clean(
        r#"declare -A dict
dict['`']=2; dict["'"]=3; dict['\']=4
for k in "${!dict[@]}"; do unset -v dict["$k"]; done
declare -p dict
echo "count=${#dict[@]}"
"#,
        "declare -A dict=()\ncount=0\n",
    );
}

/// assoc9.sub function form: the same removals routed through a
/// `del` helper, additionally covering the double-quote key.
#[test]
fn issue2657_unset_via_del_function() {
    assert_clean(
        r#"loaddict()
{
    dict['"']=1
    dict['`']=2
    dict["'"]=3
    dict['\']=4
}
del()
{
	unset -v dict["$1"]
}
declare -A dict
loaddict
echo "count=${#dict[@]}"
for k in "${!dict[@]}"; do del "$k"; done
declare -p dict
echo "count=${#dict[@]}"
"#,
        "count=4\ndeclare -A dict=()\ncount=0\n",
    );
}

/// Linked chain: five keys (backtick, ', \, ", space) removed one at
/// a time through the expanded form, asserting the element count
/// after every single removal so a silently-kept element fails at
/// its own step.
#[test]
fn issue2657_unset_stepwise_chain_all_quote_class_keys() {
    assert_clean(
        r#"declare -A dict
dict['`']=2; dict["'"]=3; dict['\']=4; dict['"']=1; dict[' ']=5
echo "count=${#dict[@]}"
k="'"; unset -v dict["$k"]; echo "after-squote count=${#dict[@]}"
k='\'; unset -v dict["$k"]; echo "after-backslash count=${#dict[@]}"
k='`'; unset -v dict["$k"]; echo "after-backtick count=${#dict[@]}"
k='"'; unset -v dict["$k"]; echo "after-dquote count=${#dict[@]}"
k=' '; unset -v dict["$k"]; echo "after-space count=${#dict[@]}"
declare -p dict
"#,
        "count=5\nafter-squote count=4\nafter-backslash count=3\nafter-backtick count=2\nafter-dquote count=1\nafter-space count=0\ndeclare -A dict=()\n",
    );
}

/// Same removals under `shopt -s assoc_expand_once` (the assoc9.sub
/// second half): the verbatim-subscript mode must agree.
#[test]
fn issue2657_unset_with_assoc_expand_once() {
    assert_clean(
        r#"shopt -s assoc_expand_once
declare -A dict
dict['`']=2; dict["'"]=3; dict['\']=4
for k in "${!dict[@]}"; do unset -v dict["$k"]; done
declare -p dict
echo "count=${#dict[@]}"
"#,
        "declare -A dict=()\ncount=0\n",
    );
}

/// assoc9.sub quoted-subscript form: `unset -v 'b[$x]'` — not a
/// W_ARRAYREF word, but for an associative variable GNU still
/// expands the subscript once at unbind time (arrayfunc.c
/// unbind_array_element), removing the \, " and ) keys and keeping
/// the backtick and ] keys added afterwards.
#[test]
fn issue2657_unset_quoted_subscript_expands_for_assoc() {
    assert_clean(
        r#"typeset -A a b
a['\']=; a['"']=; a[')']=
for x in "${!a[@]}"; do b[$x]=; done
b['`']=; b[']']=
echo "bcount=${#b[@]}"
for x in "${!a[@]}"; do
  unset -v 'b[$x]'
done
echo "bcount=${#b[@]}"
k='`'; echo "has-backtick=${b[$k]+set}"
k=']'; echo "has-rbracket=${b[$k]+set}"
"#,
        "bcount=5\nbcount=2\nhas-backtick=set\nhas-rbracket=set\n",
    );
}

/// Guard: genuinely invalid operands keep failing in GNU — `1bad`
/// and the unterminated `dict[` are "not a valid identifier" (rc 1),
/// while a missing element and a whole-array unset succeed silently.
#[test]
fn issue2657_unset_invalid_operands_still_error() {
    assert_diag(
        r#"declare -A dict
dict[x]=1
unset -v '1bad'
echo "rc1=$?"
unset -v 'dict[missing]'
echo "rc2=$?"
unset -v 'dict['
echo "rc3=$?"
unset -v dict
echo "rc4=$?"
echo "dict-set=${dict+set}"
"#,
        "rc1=1\nrc2=0\nrc3=1\nrc4=0\ndict-set=\n",
        0,
        &[
            ("`1bad': not a valid identifier", 1),
            ("`dict[': not a valid identifier", 1),
        ],
    );
}

/// Guard: the single-quoted LITERAL forms are refused by GNU too —
/// inside one quoted word the `[` carries no W_ARRAYREF flag, and
/// the flag-0 tokenize scan fails on the escaped bracket, the
/// backtick and the unterminated quote. All three elements survive.
/// (A fix that merely loosens the subscript validator would wrongly
/// turn this cell green-by-removal.)
#[test]
fn issue2657_unset_single_quoted_literal_forms_still_error() {
    assert_diag(
        r#"declare -A dict
dict['\']=4; dict['`']=2; dict['"']=1
unset -v 'dict[\]'
echo "rc-backslash=$?"
unset -v 'dict[`]'
echo "rc-backtick=$?"
unset -v 'dict["]'
echo "rc-dquote=$?"
echo "count=${#dict[@]}"
"#,
        "rc-backslash=1\nrc-backtick=1\nrc-dquote=1\ncount=3\n",
        0,
        &[
            ("`dict[\\]': not a valid identifier", 1),
            ("`dict[`]': not a valid identifier", 1),
            ("`dict[\"]': not a valid identifier", 1),
        ],
    );
}

/// Regression lock: indexed-array element removal (quoted and bare
/// operand forms) is untouched by the assoc key handling.
#[test]
fn issue2657_unset_indexed_elements_unchanged() {
    assert_clean(
        r#"a=(x y z)
unset -v 'a[0]'
declare -p a
unset -v a[2]
declare -p a
"#,
        "declare -a a=([1]=\"y\" [2]=\"z\")\ndeclare -a a=([1]=\"y\")\n",
    );
}
