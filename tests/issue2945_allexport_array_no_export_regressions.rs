//! rubash#2945 — `set -a` wrongly marks arrays `-x`.
//!
//! GNU anchor: mark_modified_vars (flags.c:47,167) is applied ONLY by the
//! scalar bind primitives — bind_variable_internal (variables.c:3211-3212),
//! bind_variable_value (variables.c:3366-3367) and bind_function
//! (variables.c:3480-3481). Array creation and compound assignment never
//! pass through them: make_new_array_variable (variables.c:2798),
//! make_new_assoc_variable (variables.c:2851) and the compound path
//! do_compound_assignment (subst.c:3441) -> assign_array_from_string
//! (arrayfunc.c:507) set no export bit, and a subscript bind
//! (assign_array_element, arrayfunc.c:334) does not either. So under
//! `set -a` a compound-assigned array stays unexported while a plain
//! scalar assignment in the same shell IS exported.
//!
//! One GNU quirk is pinned as a CONTROL, not a bug: `declare arr=(1 2)`
//! with NO -a/-A option at top level shows -ax, because declare's valued
//! word first creates the variable through bind_variable
//! (ASS_FORCE|V_NOFREE, variables.c:~3340) — which marks it — before the
//! compound conversion. Inside a function the same form shows no -x
//! (the local creation path performs no such bind); that function form
//! is pre-existing behaviour outside this issue's morphs and is not
//! pinned here.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (oracle probes on
//! master 4f747412): the compound/declare -a/declare -A/append facets
//! diverge; the controls agree on both shells.

use std::process::{Command, Stdio};

fn run_c(script: &str) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash -c");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn compound_indexed_assignment_not_exported_under_allexport() {
    // Regression seed (mC1): plain compound assignment binds through
    // do_compound_assignment, which never marks.
    let (_, stdout, _) = run_c("set -a; arr=(1 2); declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"1\" [1]=\"2\")\n");
    // The seed's upper-case form takes the same compound path.
    let (_, stdout, _) = run_c("set -a; ARR=(1 2); declare -p ARR");
    assert_eq!(stdout, "declare -a ARR=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn compound_assoc_assignment_not_exported_under_allexport() {
    // Regression seed (mC2): declare -A routes creation through
    // make_new_assoc_variable, which never marks.
    let (_, stdout, _) = run_c("set -a; declare -A m=([k]=v); declare -p m");
    assert_eq!(stdout, "declare -A m=([k]=\"v\" )\n");
}

#[test]
fn empty_compound_assignment_not_exported_under_allexport() {
    // Regression seed (mC3): even an empty compound creates the array
    // unmarked.
    let (_, stdout, _) = run_c("set -a; arr=(); declare -p arr");
    assert_eq!(stdout, "declare -a arr=()\n");
}

#[test]
fn declare_a_compound_not_exported_under_allexport() {
    // Regression seed (mC5): an explicit -a creation is a
    // make_new_array_variable bind, never a scalar bind.
    let (_, stdout, _) = run_c("set -a; declare -a arr=(1 2); declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn declare_a_scalar_value_not_exported_under_allexport() {
    // Same creation path: `declare -a s=1` stores element 0 through the
    // array machinery; no scalar bind_variable call marks it.
    let (_, stdout, _) = run_c("set -a; declare -a s=1; declare -p s");
    assert_eq!(stdout, "declare -a s=([0]=\"1\")\n");
}

#[test]
fn compound_append_not_exported_under_allexport() {
    // Append is still the compound path end to end.
    let (_, stdout, _) = run_c("set -a; arr=(1); arr+=(2); declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn subscript_assignment_not_exported_under_allexport() {
    // Control (mC4, both shells): an element bind never marked.
    let (_, stdout, _) = run_c("set -a; arr[0]=x; declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"x\")\n");
}

#[test]
fn scalar_assignment_exported_under_allexport() {
    // Control (mC8, both shells): the scalar bind DOES mark.
    let (_, stdout, _) = run_c("set -a; s=x; declare -p s");
    assert_eq!(stdout, "declare -x s=\"x\"\n");
}

#[test]
fn declare_compound_without_array_option_keeps_gnu_quirk() {
    // Control (both shells): flagless valued declare creates the name
    // through bind_variable before the compound conversion, so the
    // export mark GNU shows here must survive the fix.
    let (_, stdout, _) = run_c("set -a; declare arr=(1 2); declare -p arr");
    assert_eq!(stdout, "declare -ax arr=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn compound_assignment_without_allexport_unmarked() {
    // Control (mC6, both shells): no set -a, no marks anywhere.
    let (_, stdout, _) = run_c("arr=(1 2); declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"1\" [1]=\"2\")\n");
}
