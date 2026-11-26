//! rubash#3022 — pattern substitution whose replacement carries an
//! operator character is misclaimed by an operator arm when the word is
//! double-quoted.
//!
//! Repro: `foo='['; echo "${foo/[/]-}"` — GNU prints `]-` (rc 0); rubash
//! aborted the command with `/: arithmetic syntax error` (rc 1).
//!
//! Root cause, two faces of one defect:
//!
//! 1. Dispatch order. GNU `parameter_brace_expand` (subst.c:9777-9917)
//!    dispatches on the ONE character that ends the parameter name:
//!    `/` routes to pattern substitution (`want_patsub`, subst.c:9917),
//!    and no operator character inside the pattern or replacement is
//!    ever an operator candidate (name extraction itself stops at the
//!    first `/`, subst.c:9807-9813, charlist `"#%^,:-=?+/@}"` — the `/`
//!    sits between `+` and `@`). The unquoted chain
//!    (`expand_braced_parameter_word`, `expand_word.rs`) already claims
//!    the replacement arm first, but both double-quoted chains
//!    (`expand_quoted_parameter_word` and
//!    `expand_quoted_parameter_word_mut_routed`,
//!    `parameter_words.rs`) ran their operator arms first. For
//!    `${foo/[/]-}` the `-` arm split at the `-` in the replacement
//!    `]-`, re-read `foo/[/]` as an array element, and evaluated the
//!    `/` subscript arithmetically. Fix: claim
//!    `expand_braced_replacement_parameter` before the `:-` arm in
//!    both quoted chains — it only fires when the name head is
//!    immediately followed by `/`, so no operator body can be stolen.
//!
//! 2. Validity gate. The same misclaim also fired through the embedded
//!    walker's `?`/`:?` pre-scan (`parse_parameter_error_operator`,
//!    `embedded_parameters.rs`), whose guard is
//!    `is_parameter_error_name` (`parameter_ops.rs`): its array arm was
//!    a bare `parse_array_subscript(name).is_some()`, which accepts
//!    `foo/[/]` — base `foo/` — as an array reference. GNU
//!    `tokenize_array_reference` (arrayfunc.c:1297-1313) requires the
//!    base before `[` to pass `valid_identifier`. Fix: the array arm
//!    now requires a legal-identifier base, allowing the `!`-indirect
//!    form GNU accepts (`${!arr[0]-def}`) while `${#arr[0]-def}`,
//!    `${1[0]-def}` and `${@[0]-def}` stay bad substitution in GNU.
//!
//! 3. Command-word pre-scans. A whole-word `"${...}"` reaches
//!    `quoted_braced_alternate_positional_at_values` and
//!    `braced_alternate_word_values` (`command_prepare.rs`) before the
//!    quoted dispatch chains, and both split the body on `:+`/`+`/`:-`
//!    /`-` and evaluate the left half with `parameter_operator_value`
//!    — the first with no validity guard at all. That is why the repro
//!    passed in a script file (affix text keeps the word off the
//!    whole-word path) but still failed under `rubash -c` and in the
//!    pin harness. Fix: both pre-scans return None when the body
//!    parses as a pattern substitution, so the word falls through to
//!    the normal quoted expansion path. This also repairs whole-word
//!    `"${foo/bar-$@}"`, which the first pre-scan answered with the
//!    positional list instead of the patsub result.
//!
//! Oracle evidence: GNU bash 5.3.20 script-file probes
//! (`/tmp/disc3022.sh`, `/tmp/diff3022.sh`, `/tmp/probe3022c.sh`,
//! `/tmp/probe3022d.sh`) — 60-line differential byte-identical
//! (modulo `$$`), including the fatal-error aborts.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions. `LC_ALL=C.UTF-8` is set on the child explicitly so the
//! pin is hermetic under any ambient locale.

use std::process::{Command, Output, Stdio};

fn run_raw(script: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash")
        .wait_with_output()
        .expect("wait rubash")
}

fn run(script: &str) -> Vec<u8> {
    let output = run_raw(script);
    assert!(
        output.status.success(),
        "rubash -c {script:?} exited with {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "rubash -c {script:?} wrote to stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn check(script: &str, expected: &str) {
    assert_eq!(run(script), expected.as_bytes(), "rubash -c {script:?}");
}

/// #3022 repro: the `-` in the replacement `]-` must not be read as
/// the `${var-default}` operator.
#[test]
fn pattern_subst_bracket_replacement_dash() {
    check(r#"foo='['; echo "${foo/[/]-}""#, "]-\n");
}

/// Same misclaim through the `?` error operator (embedded walker's
/// `?`/`:?` pre-scan, `parse_parameter_error_operator`).
#[test]
fn pattern_subst_bracket_replacement_question() {
    check(r#"foo='['; echo "${foo/[/]?}""#, "]?\n");
}

/// Same misclaim through the `=` / `+` / `:-` / `:+` / `:?` / `:=`
/// operator arms.
#[test]
fn pattern_subst_bracket_replacement_other_operators() {
    check(r#"foo='['; echo "${foo/[/]=}""#, "]=\n");
    check(r#"foo='['; echo "${foo/[/]+}""#, "]+\n");
    check(r#"foo='['; echo "${foo/[/]:-}""#, "]:-\n");
    check(r#"foo='['; echo "${foo/[/]:+}""#, "]:+\n");
    check(r#"foo='['; echo "${foo/[/]:?}""#, "]:?\n");
    check(r#"foo='['; echo "${foo/[/]:=}""#, "]:=\n");
}

/// Global substitution with the same bracket pattern.
#[test]
fn pattern_subst_bracket_replacement_global() {
    check(r#"foo='['; echo "${foo//[/]/-}""#, "]/-\n");
    check(r#"foo='[/]'; echo "${foo//[/]/-}""#, "]/-/]\n");
}

/// The operator arms the fix must not steal: `-`, `:-`, `+`, `:+`,
/// `?`, `:?`, `=`, `:=`, `#` trim, `%` trim, substring, casemod,
/// `@Q`, length — quoted and unquoted.
#[test]
fn operator_arms_survive() {
    check(r#"a=hello; echo "${a-b}""#, "hello\n");
    // `${b-def}` with b set-but-empty yields the (empty) value, not the
    // default — only `:-` tests emptiness (GNU O2).
    check(r#"b=''; echo "${b-def}""#, "\n");
    check(r#"a=hello; echo "${a:-def}""#, "hello\n");
    check(r#"b=''; echo "${b:-def}""#, "def\n");
    check(r#"a=hello; echo "${a+b}""#, "b\n");
    check(r#"b=''; echo "${b+alt}""#, "alt\n");
    check(r#"a=hello; echo "${a:+alt}""#, "alt\n");
    check(r#"b=''; echo "${b:+alt}""#, "\n");
    check(r#"a=hello; echo "${a?err}""#, "hello\n");
    check(r#"a=hello; echo "${a:?err}""#, "hello\n");
    check(r#"a=hello; echo "${a#h}""#, "ello\n");
    check(r#"a=hello; echo "${a%o}""#, "hell\n");
    check(r#"a=hello; echo "${a:1:2}""#, "el\n");
    check(r#"a=hello; echo "${a^h}""#, "Hello\n");
    check(r#"a=hello; echo "${a^^h}""#, "Hello\n");
    check(r#"a=hello; echo "${a@Q}""#, "'hello'\n");
    check(r#"a=hello; echo "${#a}""#, "5\n");
    check(r#"a=hello; echo ${a-b}"#, "hello\n");
    check(r#"b=''; echo ${b-def}"#, "\n");
}

/// Array-element operator arms keep working (`arr[0]`, `arr[@]`,
/// missing element falls back to the default).
#[test]
fn array_element_operator_arms_survive() {
    check(r#"arr=(zero one two); echo "${arr[1]-def}""#, "one\n");
    check(r#"arr=(zero one two); echo "${arr[9]-def}""#, "def\n");
    check(r#"arr=(zero one two); echo "${arr[1]:-def}""#, "one\n");
    check(r#"arr=(zero one two); echo "${arr[0]+alt}""#, "alt\n");
}

/// Indirect-through-array operator form GNU accepts
/// (`${!arr[0]-def}` indirects through `arr[0]` = `aa`, then through
/// the variable named `aa`, which is unset).
#[test]
fn indirect_array_operator_arm_survives() {
    check(r#"arr=(aa bb); echo "${!arr[0]-def}""#, "def\n");
    check(r#"arr=(aa bb); x='arr[0]'; echo "${!x-def}""#, "aa\n");
}

/// Pattern substitution on an array element and on `[@]`/`[*]`.
#[test]
fn array_element_pattern_subst_survives() {
    check(r#"arr=(zero one two); echo "${arr[1]/o/0}""#, "0ne\n");
    check(
        r#"arr=(zero one two); echo "${arr[@]/o/0}""#,
        "zer0 0ne tw0\n",
    );
    check(
        r#"arr=(zero one two); echo "${arr[*]/o/0}""#,
        "zer0 0ne tw0\n",
    );
}

/// Positional / special-parameter pattern substitution.
#[test]
fn positional_pattern_subst_survives() {
    check(r#"set -- one two three; echo "${1/o/0}""#, "0ne\n");
    check(
        r#"set -- one two three; echo "${@/o/0}""#,
        "0ne tw0 three\n",
    );
    check(
        r#"set -- one two three; echo "${*/o/0}""#,
        "0ne tw0 three\n",
    );
    check(r#"set -- one two three; echo "${#/x/y}""#, "3\n");
}

/// The unquoted whole-word chain already claimed the replacement arm
/// first — it must stay green.
#[test]
fn unquoted_pattern_subst_survives() {
    check(r#"foo='['; echo ${foo/[/]-}"#, "]-\n");
    check(r#"a=hello; echo ${a/l/-}"#, "he-lo\n");
    check(r#"arr=(zero one two); echo ${arr[1]/o/0}"#, "0ne\n");
}

/// A whole-word `${...}` reaches the command-word pre-scans
/// (`quoted_braced_alternate_positional_at_values`,
/// `braced_alternate_word_values`) before the quoted dispatch chains:
/// the `$@` in a patsub pattern/replacement must expand as patsub
/// text, not as an operator alternate.
#[test]
fn whole_word_pattern_subst_with_positional_at() {
    check(
        r#"set -- one two; foo='aXb'; echo "${foo/bar-$@}""#,
        "aXb\n",
    );
    check(
        r#"set -- one two; foo='['; echo "${foo/[/]-$@}""#,
        "]-one two\n",
    );
    check(
        r#"set -- one two; foo='['; echo "${foo/[/]-$*}""#,
        "]-one two\n",
    );
}

/// Non-identifier array bases are bad substitution in GNU, not an
/// operator form: `#`, digit and `@` bases.
#[test]
fn non_identifier_array_base_is_bad_substitution() {
    for script in [
        r#"arr=(aa bb); echo "${#arr[0]-def}""#,
        r#"set -- one two; echo "${1[0]-def}""#,
        r#"arr=(aa bb); echo "${@[0]-def}""#,
        r#"arr=(aa bb); echo "${arr[0][0]-def}""#,
    ] {
        let output = run_raw(script);
        assert!(
            !output.status.success(),
            "rubash -c {script:?} should fail, exited with {}",
            output.status
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("bad substitution"),
            "rubash -c {script:?} stderr should name bad substitution, got: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// A missing array element under `?` still reports the error and
/// aborts the command (GNU `arr[9]: err`).
#[test]
fn missing_array_element_question_still_errors() {
    let output = run_raw(r#"arr=(zero one two); echo "${arr[9]?err}""#);
    assert!(
        !output.status.success(),
        "rubash -c should fail, exited with {}",
        output.status
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("arr[9]: err"),
        "stderr should carry `arr[9]: err`, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
