//! rubash#1147 — ${!assoc[@]}:-default and ${!v:-default} inverted
//! vs GNU.
//!
//! GNU subst.c parameter_brace_expand: for an EMPTY associative array,
//! `${!assoc[@]:-default}` yields the default (the key list is empty),
//! while for a scalar `v` that is set-but-empty, `${!v:-default}` is
//! an expansion error — `!` indirects through the VALUE of v (the
//! empty string), which is not a valid variable name.
//!
//! All expectations byte-verified against GNU bash 5.3.20 (`-c`
//! strings); the diagnostic is matched on its stable core.

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
fn empty_assoc_indirect_list_takes_default() {
    let (stdout, _, code) =
        rubash("declare -A assoc=(); echo \"[${!assoc[@]:-default}]\"; echo rc=$?");
    assert_eq!(stdout, "[default]\nrc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn empty_scalar_indirect_is_invalid_name_error() {
    let (stdout, stderr, code) = rubash("v=\"\"; echo \"[${!v:-default}]\"; echo rc=$?");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains(": invalid variable name"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}
