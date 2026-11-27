//! rubash#638: under `set -u` the scalar form of an array or associative
//! array is bound by **element/key 0**, not by "the array holds any element",
//! and the decision must come from the same representation the value comes
//! from.
//!
//! GNU resolves the scalar form through element "0":
//!
//! ```c
//! /* subst.c:7787-7790 parameter_brace_expand_word() */
//! else if (assoc_p (var))
//!     temp = assoc_reference (assoc_cell (var), "0");
//! else if (array_p (var))
//!     temp = array_reference (array_cell (var), 0);
//! /* array.c:657 array_reference() and assoc.c:120 assoc_reference() return
//!    NULL when that element is absent; subst.c:10162-10169
//!    parameter_brace_expand() then raises err_unboundvar (error.c:448). */
//! ```
//!
//! with the `@`/`*` forms exempt (`all_element_arrayref`), which is why
//! `${a[@]}` stays bound on an array whose only element is at index 3.
//!
//! rubash tested `!entries.is_empty()` instead, and for assoc arrays it
//! consulted a typed cell that no writer keeps in step (only `declare` and
//! `printf -v` create it; `m[k]=v`, `unset "m[k]"`, `m=(...)`, … update only
//! the mirror) — so boundness could disagree with the value it was guarding.
//!
//! Expectations come from GNU Bash script probes (5.3.20 locally; the rule is
//! fixed by the C source above, not by that version).

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    status: i32,
}

fn run(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
    }
}

/// The scalar form is unbound: GNU exits 127 with `name: unbound variable`
/// and prints nothing.
fn assert_unbound(script: &str, name: &str) {
    let out = run(script);
    assert_eq!(
        out.stdout, "",
        "an unbound scalar form must print nothing: {script} -> {:?}",
        out.stdout
    );
    assert_eq!(out.status, 127, "{script} -> status {:?}", out.status);
    assert!(
        out.stderr.contains(&format!("{name}: unbound variable")),
        "{script} -> stderr {:?}",
        out.stderr
    );
}

/// The scalar form is bound and expands to `value`.
fn assert_bound(script: &str, value: &str) {
    let out = run(script);
    assert_eq!(
        out.stdout,
        format!("[{value}]\n"),
        "{script} -> stdout {:?}",
        out.stdout
    );
    assert_eq!(out.status, 0, "{script} -> stderr {:?}", out.stderr);
}

// --------------------------------------------------------------------------
// Direction 1: previously BOUND, GNU says UNBOUND (silent difference).
// --------------------------------------------------------------------------

#[test]
fn assoc_with_only_a_named_key_is_unbound() {
    assert_unbound("set -u; declare -A m=([k]=v); echo \"[${m}]\"", "m");
}

#[test]
fn assoc_with_two_named_keys_is_unbound() {
    assert_unbound("set -u; declare -A m=([a]=1 [b]=2); echo \"[${m}]\"", "m");
}

#[test]
fn assoc_without_declare_is_bound_at_key_zero() {
    // An unmarked `m=([0]=v)` is an INDEXED array whose subscript is
    // arithmetic; element 0 exists, so the mirror decides bound.
    assert_bound("set -u; m=([0]=v); echo \"[${m}]\"", "v");
}

#[test]
fn indexed_array_without_element_zero_is_unbound() {
    assert_unbound("set -u; declare -a a=([3]=x); echo \"[${a}]\"", "a");
    assert_unbound("set -u; declare -a a=([1]=x [2]=y); echo \"[${a}]\"", "a");
    assert_unbound("set -u; a=([3]=x); echo \"[${a}]\"", "a");
}

#[test]
fn indexed_array_that_lost_element_zero_is_unbound() {
    assert_unbound(
        "declare -a a=(x y); unset \"a[0]\"; set -u; echo \"[${a}]\"",
        "a",
    );
    assert_unbound(
        "set -u; declare -a a=(x); a[3]=y; unset \"a[0]\"; echo \"[${a}]\"",
        "a",
    );
}

#[test]
fn assoc_that_lost_key_zero_is_unbound() {
    assert_unbound(
        "set -u; declare -A m=([0]=v [k]=w); unset \"m[0]\"; echo \"[${m}]\"",
        "m",
    );
    assert_unbound(
        "set -u; declare -A m=([0]=v); unset \"m[0]\"; echo \"[${m}]\"",
        "m",
    );
}

#[test]
fn emptied_assoc_is_unbound_even_with_a_stale_cell() {
    // The typed cell still holds `[k]=v` after `unset "m[k]"`; only the
    // mirror (now `()`) reflects reality.
    let out = run("declare -A m=([k]=v); unset \"m[k]\"; set -u; printf \"x[%s]\\n\" \"${m}\"");
    assert_eq!(out.stdout, "", "{:?}", out.stdout);
    assert_eq!(out.status, 127);
    assert!(
        out.stderr.contains("m: unbound variable"),
        "{:?}",
        out.stderr
    );
}

#[test]
fn reset_to_empty_is_unbound() {
    assert_unbound("set -u; declare -A m=([a]=1); m=(); echo \"[${m}]\"", "m");
    assert_unbound("declare -a a=(x); a=(); set -u; echo \"[${a}]\"", "a");
    assert_unbound("set -u; e=(); echo \"[${e}]\"", "e");
}

// --------------------------------------------------------------------------
// Direction 2: previously UNBOUND, GNU says BOUND (aborts a working script).
// --------------------------------------------------------------------------

#[test]
fn assoc_with_key_zero_after_a_bare_declare_is_bound() {
    // `declare -A m` leaves an EMPTY cell behind, and the cell is not
    // refreshed by the write, so the stale cell used to abort here.
    assert_bound("set -u; declare -A m; m[0]=v; echo \"[${m}]\"", "v");
}

#[test]
fn assoc_with_key_zero_added_by_a_write_is_bound() {
    assert_bound("set -u; declare -A m=([k]=v); m[0]=w; echo \"[${m}]\"", "w");
}

#[test]
fn assoc_key_zero_is_bound_without_declare() {
    assert_bound("set -u; declare -A m=([0]=v); echo \"[${m}]\"", "v");
    assert_bound("set -u; declare -A m=([0]=v [k]=w); echo \"[${m}]\"", "v");
}

#[test]
fn indexed_element_zero_is_bound() {
    assert_bound("set -u; declare -a a=([0]=x); echo \"[${a}]\"", "x");
    assert_bound("set -u; declare -a a=(x y); echo \"[${a}]\"", "x");
    assert_bound("set -u; declare -a a; a[0]=v; echo \"[${a}]\"", "v");
    assert_bound("set -u; declare -a a=(); a+=(v); echo \"[${a}]\"", "v");
    assert_bound("set -u; a=(x); echo \"[${a}]\"", "x");
    assert_bound("set -u; a=([0]=x); echo \"[${a}]\"", "x");
}

#[test]
fn an_empty_string_element_zero_is_still_bound() {
    // Existence, not truthiness: GNU returns the empty string, not NULL.
    assert_bound("set -u; declare -a a=([0]=\"\"); echo \"[${a}]\"", "");
    assert_bound("set -u; declare -A m=([0]=\"\"); echo \"[${m}]\"", "");
}

// --------------------------------------------------------------------------
// Controls: forms that must NOT become unbound.
// --------------------------------------------------------------------------

#[test]
fn the_all_elements_forms_are_never_unbound() {
    // all_element_arrayref exempts `@`/`*` from the nounset check.
    assert_bound("set -u; declare -a a=([3]=x); echo \"[${a[@]}]\"", "x");
    assert_bound("set -u; declare -A m=([k]=v); echo \"[${m[@]}]\"", "v");
}

#[test]
fn element_reads_are_not_the_scalar_form() {
    assert_bound("set -u; declare -a a=([3]=x); echo \"[${a[3]}]\"", "x");
}

#[test]
fn an_empty_declared_array_stays_unbound_but_its_all_elements_form_does_not() {
    assert_unbound("set -u; declare -a a=(); echo \"[${a}]\"", "a");
    assert_bound("set -u; declare -a e=(); echo \"[${e[@]}]\"", "");
}

#[test]
fn a_scalar_whose_text_looks_like_an_array_stays_bound() {
    // rubash #311: liquidprompt tools/config-from-doc.sh reads x='()' as a
    // plain scalar; only a typed array cell takes the array rule.
    assert_bound("set -u; x=\"()\"; echo \"[${x}]\"", "()");
}

#[test]
fn the_mirror_drivers_are_unchanged() {
    // declare -p and ${#m[@]} read the mirror and were already correct.
    let out = run("declare -A m=([k]=v); unset \"m[k]\"; echo \"len=${#m[@]} $(declare -p m)\"");
    assert_eq!(out.stdout, "len=0 declare -A m=()\n", "{:?}", out.stdout);

    let out = run("declare -a a=(x y); unset \"a[0]\"; echo \"len=${#a[@]} $(declare -p a)\"");
    assert_eq!(
        out.stdout, "len=1 declare -a a=([1]=\"y\")\n",
        "{:?}",
        out.stdout
    );
}

#[test]
fn declare_conversions_are_unchanged() {
    // The scalar->assoc conversion that `declare -A` performs (GNU
    // arrayfunc.c convert_var_to_assoc moves the scalar to key "0").
    let out = run("m=scalar; declare -A m; declare -p m");
    assert_eq!(
        out.stdout, "declare -A m=([0]=\"scalar\" )\n",
        "{:?}",
        out.stdout
    );

    let out = run("m=scalar; declare -A m=([k]=v); declare -p m");
    assert_eq!(
        out.stdout, "declare -A m=([k]=\"v\" )\n",
        "{:?}",
        out.stdout
    );
}

#[test]
fn an_unmarked_element_assignment_is_still_an_indexed_array() {
    let out = run("m[k]=v; declare -p m; m[k2]=w; declare -p m");
    assert_eq!(
        out.stdout, "declare -a m=([0]=\"v\")\ndeclare -a m=([0]=\"w\")\n",
        "{:?}",
        out.stdout
    );
}
