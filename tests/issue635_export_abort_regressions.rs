//! rubash#635-export follow-up: `export -a a=( [-2]=q )` reported the
//! bad-subscript diagnostic but kept running (`AFTER`, rc 0).
//!
//! GNU routes `export -a/-A name=value` through a nested `declare_builtin`
//! (builtins/setattr.def:240-268), so an out-of-range subscript takes the
//! same path as declare: arrayfunc.c:804-809 `err_badarraysub` +
//! `any_failed++` + `break`, then the assignment layer DISCARDs the command
//! list (subst.c:11169 `bash_variable_assignment_error`). The diagnostic is
//! the bare report_error form (`bash: line 1: [-2]=q: bad array
//! subscript`, no `export:`/`readonly:` segment). `-A` operands never hit
//! this path (assoc keys are strings, no arithmetic), so they still succeed.
//!
//! Rubash threads the recorded OOB word from setattr/apply.rs through
//! export_with_io/readonly_with_io to the Executor, which prints the bare
//! diagnostic and raises the same evalerror DISCARD abort execute_declare /
//! execute_local use (rubash#637 primitive, so `eval` contains it).
//!
//! Expectations were verified byte-for-byte (stdout+stderr+rc) against GNU
//! Bash 5.3.20 over the export/readonly x -a/-A x OOB matrix.

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

/// Whole-word rejection on the export/readonly route: bare diagnostic (no
/// `export:`/`readonly:` segment), nothing after it on the same line,
/// status 1, no `AFTER`.
fn assert_export_oob(script: &str, word: &str) {
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
fn export_empty_oob() {
    assert_export_oob("export -a a=( [-2]=q ); echo AFTER", "[-2]=q");
}

#[test]
fn readonly_empty_oob() {
    assert_export_oob("readonly -a a=( [-2]=q ); echo AFTER", "[-2]=q");
}

#[test]
fn export_partial_store_kept() {
    // Elements before the failure stay stored (GNU scans and breaks). Via
    // eval so the contained failure lets the outer list observe the state
    // (same-line abandon would skip the probe on both shells).
    let out = run("eval 'export -a a=( [0]=x [-2]=q [3]=z )'; echo AFTER; declare -p a");
    // No -x: the nested declare_builtin fails before applying att_export.
    assert_eq!(out.stdout, "AFTER\ndeclare -a a=([0]=\"x\")\n");
    assert_eq!(out.status, 0);
}

#[test]
fn readonly_partial_store_kept() {
    let out = run("eval 'readonly -a a=( [0]=x [-2]=q [3]=z )'; echo AFTER; declare -p a");
    assert_eq!(out.stdout, "AFTER\ndeclare -a a=([0]=\"x\")\n");
    assert_eq!(out.status, 0);
}

#[test]
fn export_later_operand_skipped() {
    // The nested-declare DISCARD aborts the whole export command: `b=ok`
    // never binds and the list stops (GNU probe: no AFTER, rc 1).
    assert_export_oob("export -a a=([-2]=q) b=ok; echo AFTER", "[-2]=q");
}

#[test]
fn readonly_later_operand_skipped() {
    assert_export_oob("readonly -a a=([-2]=q) b=ok; echo AFTER", "[-2]=q");
}

#[test]
fn export_append_oob() {
    assert_export_oob(
        "export -a a=(p); export -a a+=( [-2]=q ); echo AFTER",
        "[-2]=q",
    );
}

#[test]
fn readonly_append_oob() {
    // The second bind never reaches the subscript scan: `a` is already
    // readonly, so GNU reports `a: readonly variable` (rc 1, no AFTER).
    let out = run("readonly -a a=(p); readonly -a a+=( [-2]=q ); echo AFTER");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "bash: line 1: a: readonly variable\n");
    assert_eq!(out.status, 1);
    // A fresh-name append does reach the scan and aborts like export.
    assert_export_oob("readonly -a c+=( [-2]=q ); echo AFTER", "[-2]=q");
}

#[test]
fn export_subshell_boundary() {
    let out = run("( export -a a=( [-2]=q ); echo SUB ); echo AFTER");
    assert_eq!(out.stdout, "AFTER\n");
    assert_eq!(out.status, 0);
}

#[test]
fn readonly_subshell_boundary() {
    let out = run("( readonly -a a=( [-2]=q ); echo SUB ); echo AFTER");
    assert_eq!(out.stdout, "AFTER\n");
    assert_eq!(out.status, 0);
}

#[test]
fn export_exit_trap_sees_failure() {
    let out = run("trap 'echo TRAP' EXIT; export -a a=( [-2]=q ); echo AFTER");
    assert_eq!(out.stdout, "TRAP\n");
    assert_eq!(out.status, 1);
}

#[test]
fn assoc_operands_have_no_oob() {
    // Assoc keys are strings, never arithmetic: no bad-subscript abort on
    // either shell (both print AFTER, rc 0).
    let out = run("export -A a=( [-2]=q ); echo AFTER; declare -p a");
    assert_eq!(out.stdout, "AFTER\ndeclare -Ax a=([-2]=\"q\" )\n");
    assert_eq!(out.status, 0);
    let out = run("readonly -A a=( [-2]=q ); echo AFTER");
    assert_eq!(out.stdout, "AFTER\n");
    assert_eq!(out.status, 0);
}

#[test]
fn export_in_range_negative_still_works() {
    // Control: [-2] with max index 1 resolves to 0, overwriting [0].
    let out = run("export -a a=( [0]=x [1]=y [-2]=q ); echo AFTER; declare -p a");
    assert_eq!(out.stdout, "AFTER\ndeclare -ax a=([0]=\"q\" [1]=\"y\")\n");
    assert_eq!(out.status, 0);
}
