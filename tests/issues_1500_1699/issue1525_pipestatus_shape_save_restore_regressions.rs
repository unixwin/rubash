//! Issue rubash#1525: the PIPESTATUS cell-shape note
//! (`ShellState::pipestatus_shape`) and the status vector were not
//! saved/restored with the bindings they describe, so a temporary
//! assignment (`PIPESTATUS=99 true`) or a function-local PIPESTATUS
//! left the shape stuck at Scalar after the binding popped — the
//! next status write stayed frozen and `declare -p` rendered a
//! ghost scalar cell (`declare -- PIPESTATUS=""`).
//!
//! GNU model (variables.c:6291 set_pipestatus_array; execute_cmd.c
//! set_pipestatus_from_exit): the completion write targets the cell
//! visible at write time. For a builtin/function/null command the
//! tempenv binding is still installed at the write (variables.c
//! push_temp_var), so a scalar temp absorbs/freezes the write and
//! is then disposed, restoring the outer cell exactly; for an
//! external command the tempenv is only the child's environment
//! (variables.c assign_in_env + maybe_make_export_env), so the
//! write lands in the parent's real cell. A function's locals pop
//! (variables.c pop_var_context) before the call's own write, so
//! the write lands in the restored global cell. Same #1363-family
//! mechanism as the temporary/local residuals: the binding was
//! restored, its shape note was not.
//!
//! Expected outputs verified against GNU bash 5.3.20 (script files).

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
fn issue1525_temp_assignment_restores_absent_pipestatus() {
    // Issue case 1: the temp scalar absorbs the write; after the
    // command the cell is absent again, so the failing declare's
    // own completion write materializes ([0]="1").
    let (stdout, stderr, code) =
        rubash("PIPESTATUS=99 true\ndeclare -p PIPESTATUS\necho \"v:[$PIPESTATUS]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "v:[1]\n");
    assert!(stderr.contains("PIPESTATUS: not found"), "stderr: {stderr}");
}

#[test]
fn issue1525_local_assignment_restores_global_pipestatus() {
    // Issue case 2: the local scalar freezes writes inside the
    // frame; at pop the global cell comes back (still absent), and
    // the call's completion write materializes ([0]="0").
    let (stdout, _, code) = rubash(
        "f() { local PIPESTATUS=7; declare -p PIPESTATUS; }\nf\ndeclare -p PIPESTATUS\necho \"v:[$PIPESTATUS]\"",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -- PIPESTATUS=\"7\"\ndeclare -a PIPESTATUS=([0]=\"0\")\nv:[0]\n"
    );
}

#[test]
fn issue1525_temp_assignment_preserves_live_array_vector() {
    // A scalar temp over a live array freezes the completion
    // write: the old vector must survive untouched (the temp
    // restore must not let the write through onto the restored
    // array either). The read must come first — any intervening
    // command's own completion write replaces the vector.
    let (stdout, _, code) =
        rubash("false | true\nPIPESTATUS=99 true\necho \"w:[${PIPESTATUS[*]}]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "w:[1 0]\n");
    let (stdout, _, code) = rubash("false | true\nPIPESTATUS=99 true\ndeclare -p PIPESTATUS");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a PIPESTATUS=([0]=\"1\" [1]=\"0\")\n");
}

#[test]
fn issue1525_temp_assignment_before_external_lets_write_land() {
    // The tempenv of an external command is the child's
    // environment only; the parent's write lands normally.
    let (stdout, _, code) = rubash("false | true\nPIPESTATUS=99 /bin/true\ndeclare -p PIPESTATUS");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a PIPESTATUS=([0]=\"0\")\n");
}

#[test]
fn issue1525_compound_temp_assignment_is_absorbed() {
    // A compound temp binds as array-shaped tempenv in GNU's
    // cell terms; its completion write is absorbed with it and
    // the outer (absent) cell is restored.
    let (stdout, stderr, code) =
        rubash("PIPESTATUS=(7 8) true\ndeclare -p PIPESTATUS\necho \"v:[$PIPESTATUS]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "v:[1]\n");
    assert!(stderr.contains("PIPESTATUS: not found"), "stderr: {stderr}");
}

#[test]
fn issue1525_temp_assignment_before_function_call_restores() {
    // Inside the call the temp scalar is visible (exported) and
    // freezes inner writes; after the call the write lands in the
    // restored global cell.
    let (stdout, _, code) = rubash(
        "f() { declare -p PIPESTATUS; false; declare -p PIPESTATUS; }\nPIPESTATUS=99 f\ndeclare -p PIPESTATUS\necho \"v:[$PIPESTATUS]\"",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -x PIPESTATUS=\"99\"\ndeclare -x PIPESTATUS=\"99\"\ndeclare -a PIPESTATUS=([0]=\"0\")\nv:[0]\n"
    );
}

#[test]
fn issue1525_local_array_pipestatus_roundtrip() {
    // A local array's own completion writes land in the local
    // array (the `local` command's write rewrites the fresh local
    // to ([0]="0")); the global cell is restored at pop and the
    // call's write lands there.
    let (stdout, _, code) =
        rubash("g() { local PIPESTATUS=(7 8); declare -p PIPESTATUS; }\ng\ndeclare -p PIPESTATUS");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -a PIPESTATUS=([0]=\"0\")\ndeclare -a PIPESTATUS=([0]=\"0\")\n"
    );
}

#[test]
fn issue1525_ordinary_array_control_unchanged() {
    // Control: an ordinary array under the same temp/local shapes
    // behaved correctly before the fix and must not regress.
    let (stdout, _, code) = rubash(
        "a=(9 9)\na=99 true\ndeclare -p a\ng() { local a=(1 2); declare -p a; }\ng\ndeclare -p a",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -a a=([0]=\"9\" [1]=\"9\")\ndeclare -a a=([0]=\"1\" [1]=\"2\")\ndeclare -a a=([0]=\"9\" [1]=\"9\")\n"
    );
}
