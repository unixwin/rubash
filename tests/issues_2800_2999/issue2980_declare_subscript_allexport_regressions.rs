//! rubash#2980 — under `set -a`, a subscript operand of
//! `declare`/`typeset` must NOT acquire the allexport mark.
//!
//! GNU anchor: a subscript operand binds through
//! assign_array_element (arrayfunc.c), which never applies the
//! mark_modified_vars export mark — that mark belongs to whole-name
//! bindings through bind_variable_internal (variables.c:3211-3212).
//! rubash's execute_declare allexport post-pass (the rubash#538
//! emulation) derived the base name from `arr[0]` and marked it,
//! producing `declare -ax` where GNU prints `declare -a`; the
//! sibling execute_local post-pass already skips subscript operands.
//! The divergence covers the top level, function bodies (declare and
//! typeset share the path), the append form, assoc keys, `-g`, and a
//! valueless subscript operand against an existing array.
//!
//! Expectations probed against GNU bash 5.3.20 on master eff7753e /
//! b3e4ff89. Existing `-ax` pins are all whole-name shapes (flagless
//! compound `declare arr=(1 2)`, explicit `declare -x`, plain scalar
//! binds) and are pinned here as controls — none depends on the
//! subscript-operand mark.

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
fn declare_subscript_operand_is_not_marked_exported() {
    // Regression seed (filing probe, both contexts).
    let (_, stdout, _) = run_c("set -a; declare arr[0]=x; declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"x\")\n");
}

#[test]
fn typeset_subscript_operand_is_not_marked_exported() {
    let (_, stdout, _) = run_c("set -a; typeset arr[0]=x; declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"x\")\n");
}

#[test]
fn declare_subscript_append_operand_is_not_marked_exported() {
    let (_, stdout, _) = run_c("set -a; declare arr[0]+=x; declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"x\")\n");
}

#[test]
fn declare_assoc_subscript_operand_is_not_marked_exported() {
    let (_, stdout, _) = run_c("set -a; declare -A m; declare m[k]=v; declare -p m");
    assert_eq!(stdout, "declare -A m=([k]=\"v\" )\n");
}

#[test]
fn declare_subscript_in_function_is_not_marked_exported() {
    let (_, stdout, _) = run_c("set -a; f() { declare arr[0]=x; declare -p arr; }; f");
    assert_eq!(stdout, "declare -a arr=([0]=\"x\")\n");
}

#[test]
fn declare_global_subscript_operand_is_not_marked_exported() {
    let (_, stdout, _) = run_c("set -a; declare -g arr[0]=x; declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"x\")\n");
}

#[test]
fn valueless_subscript_operand_does_not_mark_existing_array() {
    let (_, stdout, _) = run_c("arr=(1 2); set -a; declare arr[0]; declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn whole_name_marking_controls_unchanged() {
    // Flagless compound at top level keeps the -ax quirk (GNU binds
    // the name through bind_variable first, rubash#2964).
    let (_, stdout, _) = run_c("set -a; declare arr=(1 2); declare -p arr");
    assert_eq!(stdout, "declare -ax arr=([0]=\"1\" [1]=\"2\")\n");
    // Valued and valueless scalar operands still mark (rubash#538).
    let (_, stdout, _) = run_c("set -a; declare v=x; declare -p v");
    assert_eq!(stdout, "declare -x v=\"x\"\n");
    let (_, stdout, _) = run_c("set -a; declare v; declare -p v");
    assert_eq!(stdout, "declare -x v\n");
    // An explicit -x flag still applies to a subscript operand.
    let (_, stdout, _) = run_c("declare -x arr[0]=x; declare -p arr");
    assert_eq!(stdout, "declare -ax arr=([0]=\"x\")\n");
    // local's subscript skip was already correct (sibling control).
    let (_, stdout, _) = run_c("set -a; f() { local arr[0]=x; declare -p arr; }; f");
    assert_eq!(stdout, "declare -a arr=([0]=\"x\")\n");
    // Without allexport nothing marks either way.
    let (_, stdout, _) = run_c("set +a; declare arr[0]=x; declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"x\")\n");
}
