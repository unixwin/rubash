//! rubash#1707 — `[[ -v nameref ]]` tested the nameref cell itself
//! (which always exists) instead of resolving to its target.
//!
//! GNU ground truth: test.c:650 unary `-v` resolves the operand with
//! find_variable, which follows nameref chains (variables.c
//! find_variable_nameref); the verdict is the TARGET's set-ness. A
//! chain ending in an unset name, an empty cell, or a cycle is false;
//! a target whose cell text carries a subscript (`declare -n r='a[1]'`)
//! is also false (-v never decomposes the target text), while an
//! operand subscript applies to the resolved target. Expectations
//! verified against GNU bash 5.3.20 (dual runs). Distinct from closed
//! #1108 (the compat `[@]` any-element form).

use std::process::{Command, Stdio};

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1707_dash_v_follows_nameref() {
    let (stdout, _, _) = run("declare -n dref=nosuchvar\n\
         [[ -v dref ]]; echo \"missing=$?\"\n\
         v=1; declare -n ref=v\n\
         [[ -v ref ]]; echo \"set=$?\"\n\
         unset v\n\
         [[ -v ref ]]; echo \"after-unset=$?\"\n");
    assert_eq!(stdout, "missing=1\nset=0\nafter-unset=1\n");
}

#[test]
fn issue1707_dash_v_nameref_chains_and_cells() {
    let (stdout, _, _) = run("declare -n dref=nosuchvar\n\
         declare -n r2=dref\n\
         [[ -v r2 ]]; echo \"chain=$?\"\n\
         declare -n emptycell\n\
         [[ -v emptycell ]]; echo \"empty-cell=$?\"\n\
         v=; declare -n r3=v\n\
         [[ -v r3 ]]; echo \"empty-string-target=$?\"\n\
         declare -n ca=cb; declare -n cb=ca\n\
         [[ -v ca ]]; echo \"circular=$?\"\n");
    assert_eq!(
        stdout,
        "chain=1\nempty-cell=1\nempty-string-target=0\ncircular=1\n"
    );
}

#[test]
fn issue1707_dash_v_nameref_array_targets() {
    let (stdout, _, _) = run("arr=(x y)\n\
         declare -n ra=arr\n\
         [[ -v ra ]]; echo \"whole-array=$?\"\n\
         [[ -v ra[1] ]]; echo \"operand-sub=$?\"\n\
         [[ -v ra[5] ]]; echo \"operand-sub-missing=$?\"\n\
         declare -n rsub='arr[1]'\n\
         [[ -v rsub ]]; echo \"target-with-sub=$?\"\n");
    assert_eq!(
        stdout,
        "whole-array=0\noperand-sub=0\noperand-sub-missing=1\ntarget-with-sub=1\n"
    );
}

#[test]
fn issue1707_dash_v_nameref_test_builtin() {
    // The `test`/`[` builtin shares test.c's -v machinery with `[[ ]]`.
    let (stdout, _, _) = run("declare -n dref=nosuchvar\n\
         [ -v dref ]; echo \"missing=$?\"\n\
         v=1; declare -n ref=v\n\
         [ -v ref ]; echo \"set=$?\"\n\
         [[ -v plain_unset ]]; echo \"plain-unset=$?\"\n\
         [[ -v v ]]; echo \"plain-set=$?\"\n");
    assert_eq!(stdout, "missing=1\nset=0\nplain-unset=1\nplain-set=0\n");
}
