//! Issue rubash#1600: a negative substring length on the `name[@]` /
//! `name[*]` spelling was rejected with `substring expression < 0`
//! based on the name's FORM, but GNU get_var_and_type
//! (subst.c parameter_brace_substring, subst.c:9050) classifies the
//! operand by the base variable's TYPE: a scalar base is VT_VARIABLE,
//! so verify_substring_values (subst.c:8395) adjusts the negative
//! length by the string length exactly like the bare-name form. Only
//! a real array base (VT_ARRAYVAR) and the positional parameters
//! (VT_POSPARMS) reject a negative length unconditionally, and an
//! unset base or an element-less array take the early NULL return and
//! never error at all.
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
fn issue1600_scalar_base_at_form_negative_length_slices_string() {
    let (stdout, stderr, code) = rubash("v='hello world'\necho \"[${v[@]:0:-1}]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[hello worl]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1600_scalar_base_star_form_negative_length_slices_string() {
    let (stdout, _, code) = rubash("v='hello world'\necho \"[${v[*]:6:-1}]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[worl]\n");
}

#[test]
fn issue1600_scalar_base_negative_offset_and_length() {
    let (stdout, _, code) = rubash("v='hello world'\necho \"[${v[@]: -3:-1}]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[rl]\n");
}

#[test]
fn issue1600_scalar_base_still_errors_when_end_precedes_start() {
    // v has 11 chars; :0:-20 adjusts the end to -9, which precedes the
    // start, so verify_substring_values still rejects it.
    let (stdout, stderr, code) = rubash("v='hello world'\necho \"[${v[@]:0:-20}]\"\necho after");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert!(
        stderr.contains("-20: substring expression < 0"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1600_real_array_base_negative_length_still_errors() {
    let (stdout, stderr, code) = rubash("a=(x y)\necho \"[${a[@]:0:-1}]\"\necho after");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert!(
        stderr.contains("-1: substring expression < 0"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1600_empty_array_base_negative_length_is_silent() {
    let (stdout, stderr, code) = rubash("a=()\necho \"[${a[@]:0:-1}]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1600_unset_base_negative_length_is_silent() {
    let (stdout, stderr, code) = rubash("echo \"[${never_set_xyz[@]:0:-1}]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1600_set_empty_scalar_base_negative_length_errors() {
    // A set-but-empty scalar DOES reach verify_substring_values: the
    // adjusted end (0 + -1) precedes the start.
    let (stdout, stderr, code) = rubash("v=''\necho \"[${v[@]:0:-1}]\"\necho after");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert!(
        stderr.contains("-1: substring expression < 0"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue1600_array_storage_shaped_scalar_is_sliced_as_string() {
    // The verdict is the cell type, not the value's text (rubash#755):
    // a scalar whose text looks like array storage slices as a string.
    let (stdout, _, code) = rubash("x='(a b c)'\necho \"[${x[@]:0:-1}]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[(a b c]\n");
}

#[test]
fn issue1600_positional_negative_length_still_errors() {
    let (stdout, stderr, code) = rubash("echo \"[${@:0:-1}]\"\necho after");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert!(
        stderr.contains("-1: substring expression < 0"),
        "stderr: {stderr}"
    );
}
