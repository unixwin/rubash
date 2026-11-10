//! rubash#3024 (eco-2906 M6) — `${foo:t5:h2:l}` with `foo` unset must
//! short-circuit to empty; rubash evaluated the offset/length arithmetic
//! and errored.
//!
//! GNU anchor: subst.c:9050-9057 `parameter_brace_substring` returns NULL
//! before `verify_substring_values` is ever called when the parameter's
//! value is NULL (unset) and the name is not exactly `@` or `*`. The
//! caller (subst.c:10179-10203) treats the NULL result as an empty
//! expansion with no diagnostic. A set-but-null variable (`foo=""`) has
//! a non-NULL value, so its offsets ARE evaluated and bad offsets still
//! error (subst.c:8395 `verify_substring_values` -> evalexp). The `@`/`*`
//! names are explicitly exempt from the early return, so `${@:t5:h2:l}`
//! with no positional parameters still errors. The nounset check
//! (subst.c:10156) precedes all of this, so `set -u` still reports the
//! unbound variable.
//!
//! The short-circuit is total: the offset text is never expanded, so a
//! command substitution or `$((i++))` inside the offsets of an unset
//! variable never executes (GNU probes, 2026-11-08).
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions plus exit code and stderr-substring assertions.
//! `LC_ALL=C.UTF-8` is set on the child explicitly so the pin is
//! hermetic under any ambient locale.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

/// Unset variable + bad offsets: empty expansion, rc 0, silent.
fn expect_empty(script: &str, ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.stdout,
        b"XY\n",
        "{ctx}: stdout must be just the affixes; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        out.code,
        0,
        "{ctx}: rc must be 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Bad offsets on a SET variable (or `@`/`*`): rc 1 with GNU's diagnostic.
fn expect_arith_error(script: &str, needle: &str, ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.code,
        1,
        "{ctx}: rc must be 1; stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(needle),
        "{ctx}: stderr must contain {needle:?}; got {stderr}"
    );
}

#[test]
fn unset_bad_offsets_short_circuits_to_empty() {
    expect_empty(
        "unset foo; echo \"X${foo:t5:h2:l}Y\"",
        "repro: unset foo, ${foo:t5:h2:l}",
    );
}

#[test]
fn unset_bad_offset_only_short_circuits() {
    expect_empty(
        "unset foo; echo \"X${foo:bad}Y\"",
        "unset foo, bad offset only",
    );
}

#[test]
fn unset_bad_length_only_short_circuits() {
    expect_empty(
        "unset foo; echo \"X${foo:1:bad}Y\"",
        "unset foo, bad length only",
    );
}

#[test]
fn unset_empty_offset_bad_length_short_circuits() {
    expect_empty(
        "unset foo; echo \"X${foo::bad}Y\"",
        "unset foo, empty offset + bad length",
    );
}

#[test]
fn unset_offset_with_variable_short_circuits() {
    expect_empty(
        "unset foo; bar=3; echo \"X${foo:bar+bad}Y\"",
        "unset foo, offset mentions a set variable",
    );
}

#[test]
fn unset_offset_comsub_never_executes() {
    // The offset text is never expanded for an unset variable: the
    // command substitution must not run at all (GNU: no MARKER on stderr).
    let out = run("unset foo; echo \"X${foo:$(echo MARKER >&2)}Y\"");
    assert_eq!(
        out.stdout,
        b"XY\n",
        "unset foo, comsub offset: stdout must be just the affixes; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(out.code, 0, "unset foo, comsub offset: rc must be 0");
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("MARKER"),
        "unset foo, comsub offset: the offset comsub must not execute; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn unset_offset_arith_side_effect_never_runs() {
    // `$((i++))` inside the offsets of an unset variable must not
    // increment: GNU leaves i at 0.
    let out = run("unset foo; i=0; echo \"X${foo:$((i++))}Y\"; echo \"i=$i\"");
    assert_eq!(
        out.stdout,
        b"XY\ni=0\n",
        "unset foo, $((i++)) offset: got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(out.code, 0, "unset foo, $((i++)) offset: rc must be 0");
}

#[test]
fn unset_array_element_bad_offsets_short_circuits() {
    expect_empty(
        "unset arr; echo \"X${arr[0]:t5:h2:l}Y\"",
        "unset array element arr[0], bad offsets",
    );
}

#[test]
fn unset_whole_array_bad_offsets_short_circuits() {
    expect_empty(
        "unset arr; echo \"X${arr[@]:t5:h2:l}Y\"",
        "unset whole array arr[@], bad offsets",
    );
}

#[test]
fn unset_assoc_element_bad_offsets_short_circuits() {
    expect_empty(
        "unset m; declare -A m; echo \"X${m[k]:t5:h2:l}Y\"",
        "unset assoc element m[k], bad offsets",
    );
}

#[test]
fn unset_positional_bad_offsets_short_circuits() {
    expect_empty(
        "set --; echo \"X${1:t5:h2:l}Y\"",
        "unset positional $1, bad offsets",
    );
}

#[test]
fn indirect_unset_target_bad_offsets_short_circuits() {
    expect_empty(
        "unset foo bar; foo=bar; echo \"X${!foo:t5:h2:l}Y\"",
        "indirect ${!foo} with unset target, bad offsets",
    );
}

#[test]
fn unset_numeric_offsets_stay_empty() {
    expect_empty(
        "unset foo; echo \"X${foo:1:2}Y\"",
        "control: unset foo, numeric offsets",
    );
}

#[test]
fn set_null_bad_offsets_still_error() {
    expect_arith_error(
        "foo=\"\"; echo \"X${foo:t5:h2:l}Y\"",
        "foo: h2:l: arithmetic syntax error in expression (error token is \":l\")",
        "set-but-null foo, bad offsets must still error",
    );
}

#[test]
fn set_bad_offsets_still_error() {
    expect_arith_error(
        "foo=hello; echo \"X${foo:t5:h2:l}Y\"",
        "foo: h2:l: arithmetic syntax error in expression (error token is \":l\")",
        "set foo, bad offsets must still error",
    );
}

#[test]
fn set_positional_bad_offsets_still_error() {
    expect_arith_error(
        "set -- hello; echo \"X${1:t5:h2:l}Y\"",
        "1: h2:l: arithmetic syntax error in expression (error token is \":l\")",
        "set $1, bad offsets must still error",
    );
}

#[test]
fn set_assoc_element_bad_offsets_still_error() {
    expect_arith_error(
        "declare -A m=([k]=v); echo \"X${m[k]:t5:h2:l}Y\"",
        "m[k]: h2:l: arithmetic syntax error in expression (error token is \":l\")",
        "set assoc element m[k], bad offsets must still error",
    );
}

#[test]
fn indirect_set_target_bad_offsets_still_error() {
    expect_arith_error(
        "foo=bar; bar=hello; echo \"X${!foo:t5:h2:l}Y\"",
        "!foo: h2:l: arithmetic syntax error in expression (error token is \":l\")",
        "indirect ${!foo} with set target, bad offsets must still error",
    );
}

#[test]
fn count_param_bad_offsets_still_error() {
    expect_arith_error(
        "set -- a b; echo \"X${#:t5:h2:l}Y\"",
        "#: h2:l: arithmetic syntax error in expression (error token is \":l\")",
        "set $#, bad offsets must still error",
    );
}

// NOTE (residual, filed separately — NOT part of #3024): `${?}` substring
// expansion is broken independently of this issue on the base tree:
// `false; echo "X${?:0:1}Y"` prints `XY` on rubash but `X1Y` on GNU, and
// `${?:t5:h2:l}` expands empty with rc 0 instead of erroring. The sibling
// specials `$`, `-`, `0`, `#` all slice correctly. The #3024 fix deliberately
// does not short-circuit `?` (its operator value is Some), so this stays
// exactly as broken as before until the residual is fixed.

#[test]
fn at_param_bad_offsets_still_error() {
    expect_arith_error(
        "set --; echo \"X${@:t5:h2:l}Y\"",
        "@: h2:l: arithmetic syntax error in expression (error token is \":l\")",
        "${@:t5:h2:l} with no positionals is exempt from the short-circuit and must still error",
    );
}

#[test]
fn star_param_bad_offsets_still_error() {
    expect_arith_error(
        "set --; echo \"X${*:t5:h2:l}Y\"",
        "*: h2:l: arithmetic syntax error in expression (error token is \":l\")",
        "${*:t5:h2:l} with no positionals is exempt from the short-circuit and must still error",
    );
}

#[test]
fn nounset_unset_bad_offsets_still_unbound() {
    // GNU subst.c:10156: the unbound check precedes the substring
    // evaluation, so `set -u` reports the unbound variable (rc 127),
    // never the arithmetic error and never an empty expansion.
    let out = run("set -u; unset foo; echo \"X${foo:t5:h2:l}Y\"");
    assert_eq!(
        out.code,
        127,
        "set -u, unset foo: rc must be 127; stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("foo: unbound variable"),
        "set -u, unset foo: stderr must report the unbound variable; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn valid_substring_still_slices() {
    let out = run("foo=hello; echo \"${foo:1:3}\"");
    assert_eq!(out.stdout, b"ell\n", "valid substring must still slice");
    assert_eq!(out.code, 0, "valid substring rc must be 0");
}
