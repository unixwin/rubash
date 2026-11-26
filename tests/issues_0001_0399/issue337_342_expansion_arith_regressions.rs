//! Issues rubash#337/#338/#341/#342 — expansion and arithmetic singles.
//!
//! #337: `set -- {a,}` yielded argc=2 — the empty brace alternative
//! survived as a positional. GNU braces.c brace_expand emits the empty
//! word, and expand_word_list_internal (subst.c:12026-12035) discards an
//! unquoted null result the same way it discards an unset variable's.
//! #338: `${!v@}` was `bad substitution` — invalid_at_transform_base
//! misread the bang-prefix LIST marker (`!v` + empty `@`) as an empty
//! `@` transform on a set variable. GNU subst.c:9978-10007 decides
//! `${!PREFIX@}`/`${!PREFIX*}` (variable-name list, `@` one field per
//! name, `*` IFS[0]-joined) BEFORE parameter_brace_transform.
//! #341: `x=010; $((x))` gave 10 — GNU expr.c:1539/1557 strlong re-parses
//! `0nnn` variable values as base 8.
//! #342: `a **= 5` evaluated (32) — GNU expr.c:44's assignment-operator
//! table has no `**=`; the tokenizer yields `**` and the `=` lands in
//! operand position: `arithmetic syntax error: operand expected (error
//! token is "= 5")`.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

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

/// #337: the empty brace alternative produces no positional word.
#[test]
fn empty_brace_member_drops_null_word() {
    let (stdout, stderr, code) =
        rubash("set -- {a,}\necho \"argc=$#\"\nset -- {,a}\necho \"argc=$#\"");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "argc=1\nargc=1\n");
}

/// #337 guards: a two-member brace still yields two words, and a
/// non-empty-bracketed empty member stays ([a] []).
#[test]
fn brace_expansion_keeps_nonempty_members() {
    let (stdout, _, code) = rubash("set -- {a,b}\necho \"argc=$#\"\necho [{a,}]");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "argc=2\n[a] []\n");
}

/// #337 guard: a QUOTED empty word still produces an empty field.
#[test]
fn quoted_empty_word_stays_a_field() {
    let (stdout, _, code) = rubash("set -- a \"\"\necho \"argc=$#\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "argc=2\n");
}

/// #338: `${!v@}` lists variable names beginning with the prefix.
#[test]
fn bang_prefix_at_lists_variable_names() {
    let (stdout, stderr, code) = rubash(
        "v=w\nw=inner\necho \"${!v}\"\necho \"${!v@}\"\necho \"${!v*}\"\nq=v\necho \"${!q@}\"",
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "inner\nv\nv\nq\n");
}

/// #338 guard: `${x@Q}`-style invalid transforms on SET variables are
/// still bad substitution (the list-marker exemption must not swallow
/// them).
#[test]
fn invalid_at_transform_still_rejected() {
    let (stdout, stderr, code) = rubash("x=5\necho \"${x@}\"");
    assert_ne!(stdout, "5\n");
    assert!(
        stderr.contains("${x@}: bad substitution"),
        "stderr: {stderr}"
    );
    // Script-file probe: rc=1 (GNU). The -c harness exits 127 on the
    // aborted driver; pin only failure.
    assert_ne!(code, Some(0));
}

/// #341: leading-zero variable values re-parse as octal.
#[test]
fn octal_variable_value_in_arithmetic() {
    let (stdout, stderr, code) = rubash("x=010\necho $((x))\ni=010\necho $((i++)) $((i))");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "8\n8 9\n");
}

/// #341 guards: decimal and hex values keep their existing behavior.
#[test]
fn decimal_and_hex_variable_values_unchanged() {
    let (stdout, _, code) = rubash("d=10\necho $((d))\nh=0x10\necho $((h))");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "10\n16\n");
}

/// #342: `**=` is a syntax error with GNU's exact diagnostic.
#[test]
fn pow_assignment_operator_is_syntax_error() {
    let (stdout, stderr, code) = rubash("a=2\necho $((a **= 5))");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains(
            "a **= 5: arithmetic syntax error: operand expected (error token is \"= 5\")"
        ),
        "stderr: {stderr}"
    );
}

/// #342 guard: plain `**` exponentiation keeps working.
#[test]
fn pow_operator_still_works() {
    let (stdout, _, code) = rubash("echo $((2 ** 5))");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "32\n");
}
