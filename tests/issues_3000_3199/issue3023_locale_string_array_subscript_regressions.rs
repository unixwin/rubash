//! rubash#3023 — an array subscript written as `$"..."` was not evaluated
//! as an arithmetic expression.
//!
//! Root cause: raw subscript text bypasses the lexer, which is the place
//! that cooks `$"..."` by the same rules as `"..."` (`lexer/quotes.rs`
//! `$"` arm). The two raw-text subscript converters never learned that
//! the dollar is quote-syntax markup, so it glued onto the content:
//! `arith_subscript_text` turned `$"x y"` into `$x y` (silent index 0 —
//! `$x` unset, then bare `y` as an unset variable), `$"1+2"` into `$1+2`
//! (index 2), and `$"$(...)"` into `$$(...)` (PID + literal); the
//! subscript walker in `expand_subscript_string` resolved `${a[$"1+2"]}`
//! to `$"1+2` (closing quote eaten by the walker's `"` toggle) and assoc
//! keys stored the literal `$"..."` spelling.
//!
//! Fix: `arithmetic::decode_locale_quote_spans` rewrites each `$"..."` span
//! to `"..."`, dropping only the dollar — applied in `arith_subscript_text`
//! (write path), in `expand_subscript_string_inner` (read path + assoc
//! keys), and at the two `expand_arithmetic_special_parameters` call sites
//! in `array_assignment_exec.rs` (single-quoted raw subscripts).
//!
//! GNU anchors: `arrayfunc.c:1360` `array_expand_index` (subscript ->
//! `expand_arith_string` -> `evalexp`); `subst.c:11107`
//! `expand_array_subscript` (subscript -> `expand_subscript_string`);
//! `subst.c` `$"..."` expands exactly like `"..."` in the C locale (the
//! dollar is part of the quote syntax). Oracle: GNU bash 5.3.20, probed
//! from script files (see `~/workspace/fix-3023/probes-*.sh`).
//!
//! Residuals (same error class + rc, different diagnostic token — NOT
//! this issue): a `$'...'` span in a subscript still leaks its `$` into
//! the evalexp token (`a[$'x y']` reports token `$'x y'`, GNU `'x y'`);
//! `$"..."` inside `$((...))` bodies and `${...}` default words goes
//! through the shared embedded-parameter walker, which has no `$"` arm
//! (`$(( $"1+2" ))` errors on rubash, prints 3 on GNU;
//! `${q-$"a b"}` prints ` b` on rubash, `a b` on GNU). Both are the same
//! walker mechanism in other faces and are filed separately:
//! the `$'` token leak is #3085, the walker `$"` gap is #3086.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout /
//! stderr / status assertions. `LC_ALL=C.UTF-8` is set on the child
//! explicitly so the pin is hermetic under any ambient locale.

use std::process::{Command, Stdio};

fn run_full(script: &str) -> (i32, Vec<u8>, Vec<u8>) {
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let output = child.wait_with_output().expect("wait rubash");
    (
        output.status.code().unwrap_or(-1),
        output.stdout,
        output.stderr,
    )
}

fn stdout_of(script: &str) -> (i32, String) {
    let (rc, out, err) = run_full(script);
    assert!(
        err.is_empty(),
        "script {script:?} wrote unexpected stderr: {}",
        String::from_utf8_lossy(&err)
    );
    (rc, String::from_utf8_lossy(&out).into_owned())
}

fn expect_ok(script: &str, expected_stdout: &str, ctx: &str) {
    let (rc, out) = stdout_of(script);
    assert_eq!(rc, 0, "{ctx}: rc={rc}, script {script:?}");
    assert_eq!(out, expected_stdout, "{ctx}: got {out:?}");
}

fn expect_error(script: &str, needle: &str, ctx: &str) {
    let (rc, _out, err) = run_full(script);
    let err = String::from_utf8_lossy(&err).into_owned();
    assert_eq!(rc, 1, "{ctx}: rc={rc} (want 1), stderr: {err}");
    assert!(
        err.contains(needle),
        "{ctx}: stderr {err:?} does not contain {needle:?}"
    );
}

// --- Write path: the repro and its neighbours -------------------------------

#[test]
fn locale_subscript_two_word_value_errors_like_gnu() {
    expect_error(
        r#"a[$"x y"]=b"#,
        r#"x y: arithmetic syntax error in expression (error token is "y")"#,
        r#"a[$"x y"]=b must fail exactly like a["x y"]=b"#,
    );
}

#[test]
fn locale_subscript_unset_var_name_is_zero() {
    expect_ok(
        r#"a[$"xy"]=b; declare -p a"#,
        "declare -a a=([0]=\"b\")\n",
        r#"a[$"xy"]=b with xy unset stores element 0"#,
    );
}

#[test]
fn locale_subscript_var_name_evaluates() {
    expect_ok(
        r#"x=3; a[$"x"]=b; declare -p a"#,
        "declare -a a=([3]=\"b\")\n",
        r#"a[$"x"]=b with x=3 stores element 3"#,
    );
}

#[test]
fn locale_subscript_empty_is_zero() {
    expect_ok(
        r#"a[$""]=b; declare -p a"#,
        "declare -a a=([0]=\"b\")\n",
        r#"a[$""]=b stores element 0"#,
    );
}

#[test]
fn locale_subscript_arithmetic_word_evaluates() {
    expect_ok(
        r#"a[$"1+2"]=b; declare -p a"#,
        "declare -a a=([3]=\"b\")\n",
        r#"a[$"1+2"]=b stores element 3, not 2"#,
    );
}

#[test]
fn locale_subscript_expansion_inside_evaluates() {
    expect_ok(
        r#"y=2; a[$"x$y"]=b; declare -p a"#,
        "declare -a a=([0]=\"b\")\n",
        r#"a[$"x$y"]=b with y=2 expands to x2 (unset) -> element 0"#,
    );
}

#[test]
fn locale_subscript_comsub_runs_and_evaluates() {
    expect_ok(
        r#"a[$"$(echo 1+2)"]=b; declare -p a"#,
        "declare -a a=([3]=\"b\")\n",
        r#"a[$"$(echo 1+2)"]=b runs the comsub and stores element 3"#,
    );
}

#[test]
fn locale_subscript_param_inside_evaluates() {
    // x unset on purpose: on the base tree this errored with
    // `PID{x}` (`$$` = PID + literal `{x}`), because the raw text
    // `$"${x}"` became `$${x}`. With x set, the base tree happened
    // to take a shortcut and store element 2, so only the unset form
    // is fail-first discriminating.
    expect_ok(
        r#"a[$"${x}"]=b; declare -p a"#,
        "declare -a a=([0]=\"b\")\n",
        r#"a[$"${x}"]=b with x unset stores element 0"#,
    );
}

#[test]
fn locale_subscript_escaped_quote_errors_like_gnu() {
    expect_error(
        r#"a[$"a\"b"]=b"#,
        "invalid arithmetic operator",
        r#"a[$"a\"b"]=b must fail like GNU (a"b is not an operator)"#,
    );
}

#[test]
fn locale_subscript_mixed_with_single_quotes_errors_like_gnu() {
    expect_error(
        r#"a[$"x"'y']=b"#,
        "invalid arithmetic operator",
        r#"a[$"x"'y']=b must fail like GNU (x'y' is not an operator)"#,
    );
}

#[test]
fn locale_subscript_mid_word_spans() {
    expect_ok(
        r#"a[$"x"y]=b; declare -p a"#,
        "declare -a a=([0]=\"b\")\n",
        r#"a[$"x"y]=b: xy unset -> element 0"#,
    );
    expect_ok(
        r#"a[y$"x"]=b; declare -p a"#,
        "declare -a a=([0]=\"b\")\n",
        r#"a[y$"x"]=b: yx unset -> element 0"#,
    );
}

// --- Write path: controls that must keep working ----------------------------

#[test]
fn plain_double_quoted_subscript_still_errors() {
    expect_error(
        r#"a["x y"]=b"#,
        r#"x y: arithmetic syntax error in expression (error token is "y")"#,
        r#"a["x y"]=b must keep failing like GNU"#,
    );
}

#[test]
fn plain_single_quoted_subscript_still_errors() {
    expect_error(
        r#"a['x y']=b"#,
        r#"'x y': arithmetic syntax error: operand expected (error token is "'x y'")"#,
        r#"a['x y']=b must keep failing like GNU"#,
    );
}

#[test]
fn plain_var_subscript_still_errors() {
    expect_error(
        r#"x="y z"; a[$x]=b"#,
        r#"y z: arithmetic syntax error in expression (error token is "z")"#,
        r#"a[$x]=b with x="y z" must keep failing like GNU"#,
    );
}

#[test]
fn plain_comsub_subscript_still_evaluates() {
    expect_ok(
        r#"a["$(echo 1+2)"]=b; declare -p a"#,
        "declare -a a=([3]=\"b\")\n",
        r#"a["$(echo 1+2)"]=b must keep storing element 3"#,
    );
    expect_ok(
        r#"a[$(echo 1+2)]=b; declare -p a"#,
        "declare -a a=([3]=\"b\")\n",
        r#"a[$(echo 1+2)]=b must keep storing element 3"#,
    );
}

#[test]
fn ansi_c_subscript_still_errors_same_class() {
    // Residual, NOT #3023: the `$'` prefix still leaks into the token
    // (`$'x y'` vs GNU `'x y'`), but the error class and rc match GNU.
    expect_error(
        r#"a[$'x y']=b"#,
        "arithmetic syntax error: operand expected",
        r#"a[$'x y']=b must fail with operand expected, rc 1"#,
    );
}

// --- Read path: ${a[$"..."]} -------------------------------------------------

#[test]
fn locale_subscript_read_evaluates_arithmetic() {
    expect_ok(
        r#"a=(zero one two three); printf '%s' "${a[$"1+2"]}" "#,
        "three",
        r#"${a[$"1+2"]} reads element 3"#,
    );
}

#[test]
fn locale_subscript_read_var_name() {
    expect_ok(
        r#"a=(zero one two three); x=2; printf '%s' "${a[$"x"]}" "#,
        "two",
        r#"${a[$"x"]} with x=2 reads element 2"#,
    );
}

#[test]
fn locale_subscript_read_two_word_value_errors_like_gnu() {
    expect_error(
        r#"a=(zero one two three); printf '%s' "${a[$"x y"]}" "#,
        r#"x y: arithmetic syntax error in expression (error token is "y")"#,
        r#"${a[$"x y"]} must fail exactly like GNU"#,
    );
}

#[test]
fn locale_subscript_read_empty_is_zero() {
    expect_ok(
        r#"a=(zero one two three); printf '%s' "${a[$""]}" "#,
        "zero",
        r#"${a[$""]} reads element 0"#,
    );
}

#[test]
fn locale_subscript_read_comsub_and_param() {
    expect_ok(
        r#"a=(zero one two three); printf '%s' "${a[$"$(echo 1+2)"]}" "#,
        "three",
        r#"${a[$"$(echo 1+2)"]} reads element 3"#,
    );
    expect_ok(
        r#"a=(zero one two three); x=2; printf '%s' "${a[$"${x}"]}" "#,
        "two",
        r#"${a[$"${x}"]} with x=2 reads element 2"#,
    );
}

// --- Associative keys: the locale content is the key -------------------------

#[test]
fn locale_subscript_assoc_key_is_content() {
    let (rc, out) = stdout_of(r#"declare -A h; h[$"x y"]=b; declare -p h"#);
    assert_eq!(rc, 0, "assoc locale write rc={rc}");
    assert!(
        out.contains(r#"["x y"]="b""#),
        "assoc key must be the locale content `x y`, got {out:?}"
    );
    assert!(
        !out.contains("$\""),
        "assoc key must not carry the literal `$\"...\"` spelling, got {out:?}"
    );
}

#[test]
fn locale_subscript_assoc_var_name_key() {
    let (rc, out) = stdout_of(r#"declare -A h; h[$"x"]=c; declare -p h"#);
    assert_eq!(rc, 0, "assoc locale write rc={rc}");
    assert!(
        out.contains(r#"[x]="c""#),
        "assoc key must be `x`, got {out:?}"
    );
    assert!(
        !out.contains("$\""),
        "assoc key must not carry the literal `$\"...\"` spelling, got {out:?}"
    );
}
