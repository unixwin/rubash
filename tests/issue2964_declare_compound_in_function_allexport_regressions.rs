//! rubash#2964 — `set -a` marks a flagless `declare arr=(1 2)` INSIDE
//! a function, where GNU does not.
//!
//! GNU anchor: inside a function, declare_internal runs with the
//! local context: a compound operand is created through the local
//! array machinery and converted via do_compound_assignment
//! (subst.c:3441) with ASS_MKLOCAL — neither applies the
//! mark_modified_vars mark — and the top-level ASS_FORCE creation
//! pre-bind that produces the top-level quirk does not occur. A
//! scalar operand in the same context still binds through
//! bind_variable_value (variables.c:3320), whose tail marks at
//! variables.c:3366-3367. Rubash's declare allexport post-pass (the
//! #538 emulation in execute_declare) marked every valued operand
//! uniformly, with no function-context distinction for compound
//! values; execute_local's post-pass (#2944) already stands down for
//! compound values for exactly this reason.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (oracle probes
//! on master c095a5e2): the in-function compound facets diverge; the
//! top-level quirk, the in-function scalar, the local compound, the
//! -g compound, the quoted-paren scalar and the -a invocation agree
//! on both shells and are pinned as controls.

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
fn declare_compound_in_function_not_exported() {
    // Regression seed: the local compound conversion never marks.
    let (_, stdout, _) = run_c("set -a; f(){ declare arr=(1 2); declare -p arr; }; f");
    assert_eq!(stdout, "declare -a arr=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn typeset_compound_in_function_not_exported() {
    // typeset is the same declare_internal path.
    let (_, stdout, _) = run_c("set -a; f(){ typeset arr=(1 2); declare -p arr; }; f");
    assert_eq!(stdout, "declare -a arr=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn declare_compound_in_function_over_global_array_not_exported() {
    // The operand creates a LOCAL array shadowing the global one
    // (ASS_MKLOCAL: context differs), still through the compound
    // conversion — no mark.
    let (_, stdout, _) = run_c("arr=(0); set -a; f(){ declare arr=(9); declare -p arr; }; f");
    assert_eq!(stdout, "declare -a arr=([0]=\"9\")\n");
}

#[test]
fn declare_append_compound_in_function_not_exported() {
    // The second declare's operand is an append compound — same
    // do_compound_assignment conversion, no mark from either declare.
    let (_, stdout, _) =
        run_c("set -a; f(){ declare arr=(1); declare arr+=(2); declare -p arr; }; f");
    assert_eq!(stdout, "declare -a arr=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn declare_compound_top_level_keeps_gnu_quirk_control() {
    // Control (both shells): at top level the valued word creates
    // the name through bind_variable (ASS_FORCE) before the compound
    // conversion, and that creation bind marks — the quirk must
    // survive the fix (also pinned by #2945).
    let (_, stdout, _) = run_c("set -a; declare arr=(1 2); declare -p arr");
    assert_eq!(stdout, "declare -ax arr=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn declare_scalar_in_function_still_exported_control() {
    // Control (both shells): an in-function scalar declare binds
    // through bind_variable_value (variables.c:3366-3367), which
    // marks — the stand-down is for compound values only.
    let (_, stdout, _) = run_c("set -a; f(){ declare z=9; declare -p z; }; f");
    assert_eq!(stdout, "declare -x z=\"9\"\n");
}

#[test]
fn local_compound_in_function_not_exported_control() {
    // Control (both shells): `local` already stands down for
    // compound values in its own post-pass (#2944).
    let (_, stdout, _) = run_c("set -a; f(){ local arr=(1 2); declare -p arr; }; f");
    assert_eq!(stdout, "declare -a arr=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn declare_g_compound_in_function_still_exported_control() {
    // Control (both shells): -g forces the global context, so the
    // creation pre-bind runs and marks exactly as at top level.
    let (_, stdout, _) = run_c("set -a; f(){ declare -g arr=(1 2); }; f; declare -p arr");
    assert_eq!(stdout, "declare -ax arr=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn declare_quoted_paren_scalar_in_function_still_exported_control() {
    // Control (both shells): a QUOTED `(x)` on a scalar target is a
    // literal string bind (declare.def:935-947), not a compound
    // conversion — it marks like any scalar.
    let (_, stdout, _) = run_c("set -a; f(){ declare s=\"(x)\"; declare -p s; }; f");
    assert_eq!(stdout, "declare -x s=\"(x)\"\n");
}

#[test]
fn declare_a_compound_in_function_not_exported_control() {
    // Control (both shells): an invocation whose -a attribute ends
    // set already stands down as a whole (#2945), in functions too.
    let (_, stdout, _) = run_c("set -a; f(){ declare -a arr=(1 2); declare -p arr; }; f");
    assert_eq!(stdout, "declare -a arr=([0]=\"1\" [1]=\"2\")\n");
}
