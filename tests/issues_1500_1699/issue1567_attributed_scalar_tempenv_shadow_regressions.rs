//! Issue rubash#1567: a temporary assignment over an ATTRIBUTED
//! scalar cell (`declare -i s`, `declare -l s`, ...) evaluated the
//! window value in place under the cell's attributes, so
//! `declare -i s=5; s=007 declare -p s` printed `declare -x s="7"`
//! (leading zeros eaten by the integer evaluation) where GNU prints
//! the verbatim tempvar text.
//!
//! GNU model (variables.c:3536 assign_in_env): a non-append tempenv
//! binding is a fresh variable in the temporary_env table carrying
//! only att_exported|att_tempvar — the value text binds VERBATIM (no
//! integer evaluation, no case conversion); only the APPEND form
//! computes through the original cell (make_variable_value with
//! ASS_APPEND), and only an attribute builtin absorbing the binding
//! (declare.def:1045-1083: bind_variable with ASS_NOTEMPENV when
//! att_exported/att_readonly is applied to a tempvar) re-evaluates
//! the window text through the real cell's attributes.
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
fn issue1567_integer_window_value_is_verbatim() {
    let (stdout, stderr, code) = rubash("declare -i s=5\ns=007 declare -p s\ndeclare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"007\"\ndeclare -i s=\"5\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1567_integer_window_expression_is_verbatim() {
    let (stdout, stderr, code) = rubash("declare -i s=5\ns=3+4 declare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"3+4\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1567_integer_append_still_evaluates() {
    let (stdout, stderr, code) = rubash("declare -i s=5\ns+=7 declare -p s\ndeclare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"12\"\ndeclare -i s=\"5\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1567_case_attributes_do_not_convert_window_value() {
    let (stdout, stderr, code) = rubash("declare -l s\ns=ABC declare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"ABC\"\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = rubash("declare -u s\ns=abc declare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"abc\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1567_case_append_converts_against_the_cell() {
    let (stdout, stderr, code) = rubash("declare -l s=ABC\ns+=DEF declare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"abcdef\"\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = rubash("declare -u s=ab\ns+=cd declare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"ABCD\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1567_trace_and_valueless_cells_shadow_verbatim() {
    let (stdout, stderr, code) = rubash("declare -t s\ns=1 declare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"1\"\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = rubash("declare -i s\ns=007 declare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"007\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1567_window_value_reaches_children_and_reads() {
    let (stdout, stderr, code) = rubash("declare -i s=5\ns=007 env | grep ^s=");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "s=007\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) =
        rubash("declare -i s=5\nf() { s=007 declare -p s; }\nf\ndeclare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"007\"\ndeclare -i s=\"5\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1567_function_frame_plain_assignment_binds_the_tempvar() {
    let (stdout, stderr, code) =
        rubash("declare -i s=5\nf() { s=008; declare -p s; }\ns=1 f\ndeclare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"008\"\ndeclare -i s=\"5\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1567_nameref_window_write_shadows_the_target() {
    let (stdout, stderr, code) = rubash("declare -i s=5\ndeclare -n r=s\nr=007 declare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"007\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1567_absorbed_binding_reevaluates_through_the_cell() {
    // declare.def:1045-1083: applying att_exported to a tempvar
    // rebinds its value through the real variable, so the integer
    // attribute evaluates the window text on absorption.
    let (stdout, stderr, code) = rubash("declare -i s=5\ns=007 declare -x s\ndeclare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -ix s=\"7\"\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = rubash("declare -i s=5\ns=007 export s\ndeclare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -ix s=\"7\"\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = rubash("declare -l s=AB\ns=CdEf declare -r s\ndeclare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -rxl s=\"cdef\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1567_unabsorbed_and_fresh_absorption() {
    // `declare s` without -x/-r does not absorb the tempvar.
    let (stdout, stderr, code) = rubash("declare -i s=5\ns=007 declare s\ndeclare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -i s=\"5\"\n");
    assert_eq!(stderr, "");

    // No underlying cell: the tempvar itself becomes the binding,
    // verbatim.
    let (stdout, stderr, code) = rubash("s=007 declare -x s\ndeclare -p s");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -x s=\"007\"\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1567_readonly_window_assignment_is_dropped() {
    let (stdout, stderr, code) = rubash("declare -i r=5\nreadonly r\nr=007 echo ran\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ran\nrc=0\n");
    assert!(stderr.contains("r: readonly variable"), "stderr: {stderr}");
}
