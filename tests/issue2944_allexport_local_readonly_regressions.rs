//! rubash#2944 — `set -a` (allexport) ignored by `local`/`readonly`
//! initialization.
//!
//! GNU anchor: `set -a` sets mark_modified_vars (flags.c:47,167), and the
//! bind primitives apply it — variables.c:3211-3212 (bind_variable_internal,
//! reached by readonly's do_assignment_no_expand binds and by
//! set_var_attribute's creation of a missing variable, builtins/
//! setattr.def:651) and variables.c:3366-3367 (bind_variable_value, the
//! scalar value bind declare_internal performs for declare/typeset AND
//! local, builtins/declare.def). A valueless `local` performs no bind at
//! all (make_local_variable only), so it stays unexported; a `readonly`
//! on an existing variable performs no bind either (set_var_attribute
//! only sets the attribute), so it adds no export mark.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (oracle probes on
//! master 4f747412): the local/readonly initializer facets, the valueless
//! readonly creation facet, the readonly compound facet and the
//! `local -g` facet diverge; the controls agree on both shells.

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
fn local_initializer_exports_under_allexport() {
    // Regression seed facet: the value must cross into a child's env.
    let (_, stdout, stderr) = run_c("set -a; f(){ local z=9; printenv z; }; f");
    assert_eq!(stdout, "9\n", "stderr={stderr:?}");
    // And the attribute itself is set (GNU: declare -x z="9").
    let (_, stdout, _) = run_c("set -a; f(){ local z=9; declare -p z; }; f");
    assert_eq!(stdout, "declare -x z=\"9\"\n");
}

#[test]
fn readonly_initializer_exports_under_allexport() {
    // Regression seed facet (GNU: declare -rx r="1", printenv prints 1).
    let (_, stdout, stderr) = run_c("set -a; readonly r=1; printenv r");
    assert_eq!(stdout, "1\n", "stderr={stderr:?}");
    let (_, stdout, _) = run_c("set -a; readonly r=1; declare -p r");
    assert_eq!(stdout, "declare -rx r=\"1\"\n");
}

#[test]
fn local_integer_initializer_exports_under_allexport() {
    let (_, stdout, _) = run_c("set -a; f(){ local -i n=5; declare -p n; }; f");
    assert_eq!(stdout, "declare -ix n=\"5\"\n");
}

#[test]
fn valueless_readonly_exports_under_allexport() {
    // Same root: set_var_attribute creates a missing variable through
    // bind_variable (setattr.def:651), which marks it under set -a.
    let (_, stdout, _) = run_c("set -a; readonly r; declare -p r");
    assert_eq!(stdout, "declare -rx r\n");
}

#[test]
fn readonly_compound_initializer_exports_under_allexport() {
    // readonly's assignment word binds through do_assignment_no_expand
    // (no -a/-A rewrite), so the bind marks the array under set -a even
    // though the value itself never crosses the env boundary.
    let (_, stdout, _) = run_c("set -a; readonly arr=(1 2); declare -p arr");
    assert_eq!(stdout, "declare -arx arr=([0]=\"1\" [1]=\"2\")\n");
}

#[test]
fn local_global_initializer_exports_under_allexport() {
    // `local -g` binds the global through bind_variable_value as well.
    let (_, stdout, _) = run_c("set -a; f(){ local -g z=9; }; f; declare -p z");
    assert_eq!(stdout, "declare -x z=\"9\"\n");
}

#[test]
fn valueless_local_does_not_export_under_allexport() {
    // Control (both shells): a valueless local performs no bind
    // (make_local_variable only), so set -a does not mark it.
    let (_, stdout, _) = run_c("set -a; f(){ local z; declare -p z; }; f");
    assert_eq!(stdout, "declare -- z\n");
}

#[test]
fn readonly_on_existing_variable_adds_no_export() {
    // Control (both shells): the value was bound before set -a, and
    // set_var_attribute on an existing variable performs no new bind.
    let (_, stdout, _) = run_c("r=1; set -a; readonly r; declare -p r");
    assert_eq!(stdout, "declare -r r=\"1\"\n");
}

#[test]
fn declare_initializer_still_exports_under_allexport() {
    // Control (both shells, mA5/mA9): the declare/typeset path already
    // honored allexport before this fix.
    let (_, stdout, _) = run_c("set -a; declare z=9; declare -p z");
    assert_eq!(stdout, "declare -x z=\"9\"\n");
    let (_, stdout, _) = run_c("set -a; f(){ declare z=9; declare -p z; }; f");
    assert_eq!(stdout, "declare -x z=\"9\"\n");
}

#[test]
fn local_then_plain_assignment_exports_under_allexport() {
    // Control (both shells, mA4): the separate plain assignment binds
    // through bind_variable and was already marked.
    let (_, stdout, _) = run_c("set -a; f(){ local z; z=9; declare -p z; }; f");
    assert_eq!(stdout, "declare -x z=\"9\"\n");
}
