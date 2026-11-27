//! Issue #3609: a `local`/`declare` operand that creates a FRESH local
//! inside a function binds the new mutable instance — an outer-scope
//! readonly mark on the same name must not fire. The `nested_readonly`
//! check in the declare-local binding consulted the flat READONLY_VARS
//! name mark without regard to `locally_scoped`.
//!
//! GNU bash 5.3.20 oracle (script-file probes):
//! - `ddd(){ local -r x=0; ccc; }; ccc(){ local x=(7 8 9); ... }` —
//!   silent, binds the fresh local.
//! - `local -r x=0; local x=1` in the SAME scope — still refuses
//!   (same-scope local is readonly).
//! - `export x=v` / `declare -g x=v` inside a function on a global
//!   readonly — still refuses (non-localizing operands bind the
//!   inherited variable).
//!
//! GNU source: make_local_variable (declare.def:929+, variables.c:5524+).

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

#[test]
fn local_compound_shadowing_outer_readonly_is_silent() {
    let (stdout, stderr, code) = rubash(
        "ccc(){ local x=(7 8 9); echo \"ccc ${x[*]}\"; }\n\
         ddd(){ local -r x='0'; echo \"ddd ${x}\"; ccc; }\n\
         x=zzz; ddd; echo \"x=$x\"",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ddd 0\nccc 7 8 9\nx=zzz\n");
    assert_eq!(stderr, "");
}

#[test]
fn fresh_local_shadow_is_mutable() {
    let (stdout, stderr, code) = rubash(
        "f(){ local x=1; x=99; echo \"$x\"; }\n\
         g(){ local -r x=0; f; }\n\
         g",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "99\n");
    assert_eq!(stderr, "");
}

#[test]
fn same_scope_relocal_on_readonly_still_refuses() {
    let (_, stderr, _) = rubash("f(){ local -r x=0; local x=1; echo \"never\"; }\nf");
    assert!(
        stderr.contains("readonly"),
        "same-scope readonly local must refuse, stderr={stderr:?}"
    );
}

#[test]
fn export_on_global_readonly_inside_function_still_refuses() {
    let (stdout, stderr, _) = rubash("x=9; readonly x\nf(){ export x=1; echo \"never\"; }\nf");
    // GNU: refusal is non-fatal — diagnostic then the body continues.
    assert_eq!(stdout, "never\n");
    assert!(
        stderr.contains("readonly"),
        "export binds the global readonly, stderr={stderr:?}"
    );
}

#[test]
fn declare_g_on_global_readonly_inside_function_still_refuses() {
    let (stdout, stderr, _) = rubash("x=9; readonly x\nf(){ declare -g x=1; echo \"never\"; }\nf");
    assert_eq!(stdout, "never\n");
    assert!(
        stderr.contains("readonly"),
        "declare -g binds the global readonly, stderr={stderr:?}"
    );
}
