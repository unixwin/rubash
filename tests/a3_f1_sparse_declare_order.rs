//! Deep-audit lane A3 finding F1: `declare -p` printed sparse indexed
//! arrays in insertion order, where GNU prints ascending index order
//! (array.c:948 array_to_assign walks the element list in index order).
//!
//! GNU bash 5.3.20 oracle behavior (script-file probes, twice confirmed):
//! - `a[5]=x; a[2]=y; declare -p a` -> `declare -a a=([2]="y" [5]="x")`
//! The pre-fix build printed `([5]="x" [2]="y")`.

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
fn declare_p_sparse_array_sorts_indices() {
    let (stdout, _, code) = rubash("a[5]=x; a[2]=y; declare -p a");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a a=([2]=\"y\" [5]=\"x\")\n");
}

#[test]
fn declare_p_dense_array_unchanged() {
    let (stdout, _, code) = rubash("b[0]=p; b[1]=q; declare -p b");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a b=([0]=\"p\" [1]=\"q\")\n");
}

#[test]
fn declare_p_reassigned_sparse_array_sorts_indices() {
    let (stdout, _, code) = rubash("c[2]=y; c[5]=x; c[2]=z; declare -p c");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a c=([2]=\"z\" [5]=\"x\")\n");
}
