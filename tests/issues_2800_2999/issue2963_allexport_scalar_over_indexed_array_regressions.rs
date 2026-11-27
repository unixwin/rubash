//! rubash#2963 — `set -a` does not mark a scalar assignment over an
//! existing INDEXED array.
//!
//! GNU anchor: a scalar value bound over an existing array still goes
//! through bind_variable_internal (variables.c:3054), whose array arm
//! stores element 0 (variables.c:3195-3205) and whose tail applies the
//! mark_modified_vars mark — `if (mark_modified_vars) VSETATTR (entry,
//! att_exported)` at variables.c:3211-3212 (mark_modified_vars itself
//! is flags.c:47,167). The bind is a scalar bind for marking purposes
//! even though the value lands in element 0. The associative leg of
//! rubash's assignment path already mirrors this; the indexed leg
//! projected the storage render into the process env instead of
//! setting the export attribute.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (oracle probes
//! on master c095a5e2): the three scalar-over-indexed facets diverge;
//! the assoc twin, the pre-exported form, the append form, the
//! no-allexport form and the subscript-only form agree on both shells.

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
fn scalar_over_indexed_array_marks_exported() {
    // Regression seed: the element-0 write path of the assignment
    // executor must apply the bind tail's export mark.
    let (_, stdout, _) = run_c("set -a; arr[0]=x; arr=abde; declare -p arr");
    assert_eq!(stdout, "declare -ax arr=([0]=\"abde\")\n");
}

#[test]
fn scalar_over_compound_created_indexed_array_marks_exported() {
    // The array was created by a compound bind (unmarked, #2945);
    // the later scalar bind over it is a fresh bind_variable call
    // and marks, keeping the other elements.
    let (_, stdout, _) = run_c("set -a; arr=(1 2 3); arr=z; declare -p arr");
    assert_eq!(stdout, "declare -ax arr=([0]=\"z\" [1]=\"2\" [2]=\"3\")\n");
}

#[test]
fn scalar_over_declare_a_indexed_array_marks_exported() {
    // Same bind over an array created by `declare -a` with a sparse
    // element: element 0 is rewritten, element 3 survives, mark set.
    let (_, stdout, _) = run_c("set -a; declare -a arr; arr[3]=q; arr=zz; declare -p arr");
    assert_eq!(stdout, "declare -ax arr=([0]=\"zz\" [3]=\"q\")\n");
}

#[test]
fn scalar_over_assoc_array_marks_exported_control() {
    // Control (both shells): the assoc twin already routes through
    // the marking arm; the scalar lands at key "0", other keys kept.
    let (_, stdout, _) = run_c("set -a; declare -A m=([k]=v); m=scalar; declare -p m");
    assert_eq!(stdout, "declare -Ax m=([0]=\"scalar\" [k]=\"v\" )\n");
}

#[test]
fn scalar_over_preexported_indexed_array_stays_exported_control() {
    // Control (both shells): no set -a; the name was exported before
    // the scalar bind, and the attribute simply survives it.
    let (_, stdout, _) = run_c("arr[0]=x; export arr; arr=abde; declare -p arr");
    assert_eq!(stdout, "declare -ax arr=([0]=\"abde\")\n");
}

#[test]
fn append_over_indexed_array_marks_exported_control() {
    // Control (both shells): an append bind (`arr+=y`) is also a
    // bind_variable call, so the tail mark applies to it too.
    let (_, stdout, _) = run_c("set -a; arr[0]=x; arr+=y; declare -p arr");
    assert_eq!(stdout, "declare -ax arr=([0]=\"xy\")\n");
}

#[test]
fn scalar_over_indexed_array_without_allexport_unmarked_control() {
    // Control (both shells): no mark_modified_vars, no mark.
    let (_, stdout, _) = run_c("arr[0]=x; arr=abde; declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"abde\")\n");
}

#[test]
fn subscript_bind_alone_does_not_mark_control() {
    // Control (both shells, also pinned by #2945): an element bind
    // (assign_array_element, arrayfunc.c:334) never marks — the fix
    // must not let the scalar-over-array mark leak into it.
    let (_, stdout, _) = run_c("set -a; arr[0]=x; declare -p arr");
    assert_eq!(stdout, "declare -a arr=([0]=\"x\")\n");
}
