//! rubash#635: `declare -a a=( [-2]=q )` (builtin route) silently drops
//! out-of-range negative subscripts.
//!
//! GNU computes `max+1+ind` and, when still negative, reports on the whole
//! element word and fails without running anything after:
//!
//! ```c
//! /* arrayfunc.c:804-809 assign_compound_array_list() */
//! err_badarraysub (w); any_failed++; break;
//! /* arrayfunc.c:913-932 assign_array_var_from_string returns NULL */
//! /* subst.c:11169-11174 bash_variable_assignment_error -> DISCARD */
//! ```
//!
//! rubash's builtin resolver conflated "no `[..]` header" with "out of
//! range" (both `None`) and skipped the element, printing `AFTER` with
//! status 0. The resolver now distinguishes the cases; the scan breaks
//! with prior elements kept, and the word travels up for the fatal
//! diagnostic (report_error: no `command:` segment) plus the same DISCARD
//! abort the assignment route uses. `eval` contains it via the #637
//! primitive.
//!
//! Expectations come from the C source above; spot-checked against GNU Bash
//! 5.3.20 locally (the repo oracle is WSL 5.3.0 — no version-specific
//! behavior is asserted here).

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

/// Whole-word rejection on the builtin route: exact message, nothing after
/// it on the same line, status 1, no `AFTER`.
fn assert_builtin_oob(script: &str, word: &str) {
    let out = run(script);
    assert_eq!(out.stdout, "", "{script} -> stdout {:?}", out.stdout);
    assert_eq!(
        out.stderr,
        format!("bash: line 1: {word}: bad array subscript\n"),
        "{script} -> stderr {:?}",
        out.stderr
    );
    assert_eq!(out.status, 1, "{script} -> status {:?}", out.status);
}

#[test]
fn declare_empty_oob() {
    assert_builtin_oob("declare -a a=( [-2]=q ); echo AFTER", "[-2]=q");
}

#[test]
fn declare_partial_store_kept() {
    // Elements before the failure stay stored (GNU scans and breaks). Via
    // eval so the contained failure lets the outer list observe the state
    // (same-line abandon would skip the probe on both shells).
    let out = run("eval 'declare -a a=( [0]=x [-2]=q [1]=y )'; echo \"rc=$?\"; declare -p a");
    assert_eq!(out.stdout, "rc=1\ndeclare -a a=([0]=\"x\")\n");
    assert_eq!(out.status, 0);
}

#[test]
fn declare_append_oob() {
    assert_builtin_oob(
        "declare -a a=(p); declare -a a+=( [-2]=q ); echo AFTER",
        "[-2]=q",
    );
}

#[test]
fn declare_append_empty_phantom_gone() {
    // Previously stored a phantom `[0]=""`; GNU aborts instead.
    assert_builtin_oob(
        "declare -a a; declare -a a+=( [-2]=q ); echo AFTER",
        "[-2]=q",
    );
}

#[test]
fn local_in_function_oob() {
    assert_builtin_oob(
        "f(){ local -a a=( [-2]=q ); echo IN; }; f; echo AFTER",
        "[-2]=q",
    );
}

#[test]
fn typeset_in_function_oob() {
    assert_builtin_oob(
        "f(){ typeset -a a=( [-2]=q ); echo IN; }; f; echo AFTER",
        "[-2]=q",
    );
}

#[test]
fn eval_contained_partial() {
    // Eval catches the DISCARD via the #637 primitive: failure status,
    // partial store, caller continues.
    let out = run("eval 'declare -a a=( [0]=x [-2]=q [3]=z )'; echo AFTER; declare -p a");
    assert_eq!(out.stdout, "AFTER\ndeclare -a a=([0]=\"x\")\n");
    assert_eq!(out.status, 0);
}

#[test]
fn subshell_boundary() {
    let out = run("( declare -a a=( [-2]=q ); echo SUB ); echo AFTER");
    assert_eq!(out.stdout, "AFTER\n");
    assert_eq!(out.status, 0);
}

#[test]
fn exit_trap_sees_failure() {
    let out = run("trap 'echo TRAP' EXIT; declare -a a=( [-2]=q ); echo AFTER");
    assert_eq!(out.stdout, "TRAP\n");
    assert_eq!(out.status, 1);
}

#[test]
fn in_range_negative_still_works() {
    // Controls: in-range negatives, plain success, and the already-fixed
    // assignment route must not change.
    let out = run("declare -a a=( [0]=x [1]=y [-2]=q ); echo AFTER; declare -p a");
    assert_eq!(out.stdout, "AFTER\ndeclare -a a=([0]=\"q\" [1]=\"y\")\n");
    assert_eq!(out.status, 0);
    let out = run("declare -a a; a+=( [-2]=q ); echo AFTER");
    assert_eq!(out.status, 1);
}
