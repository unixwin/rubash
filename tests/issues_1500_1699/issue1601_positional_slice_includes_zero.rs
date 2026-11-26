//! Issue rubash#1601: positional-parameter slices resolved negative
//! offsets against the real parameters only, so `${@: -3:2}` with two
//! parameters came back empty. GNU pos_params (subst.c:3745) builds
//! the virtual list [$0, $1..$n] for every `${@:...}`/`${*:...}`
//! slice, and verify_substring_values (subst.c:8395) resolves a
//! negative offset against nargs+1: $0 occupies virtual index 0.
//! A start resolved outside the list is a silent empty result.
//!
//! Expected outputs verified against GNU bash 5.3.20 (script files).

use std::process::Command;

fn rubash(script: &str, args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .arg("probe")
        .args(args)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1601_negative_offset_slice_counts_dollar_zero() {
    // Two real parameters; -3 reaches back over $0 as well.
    let (stdout, _, code) = rubash("x=\"${@: -3:2}\"\necho \"[$x]\"", &["a", "b"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[probe a]\n");
}

#[test]
fn issue1601_negative_offset_slice_word_list() {
    // After the re-set, $1 is the slice's first element — the OLD $0.
    let (stdout, _, code) = rubash(
        "set -- \"${@: -3:2}\"\necho \"n=$# 0=$0 1=$1 2=$2\"",
        &["a", "b"],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=2 0=probe 1=probe 2=a\n");
}

#[test]
fn issue1601_negative_offset_without_length_takes_all_from_zero() {
    let (stdout, _, code) = rubash("x=\"${@: -3}\"\necho \"[$x]\"", &["a", "b"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[probe a b]\n");
}

#[test]
fn issue1601_offset_zero_slices_from_dollar_zero() {
    let (stdout, _, code) = rubash("x=\"${@:0:1}\"\necho \"[$x]\"", &["a", "b"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[probe]\n");
}

#[test]
fn issue1601_last_parameter_via_minus_one() {
    let (stdout, _, code) = rubash("x=\"${@: -1}\"\necho \"[$x]\"", &["a", "b"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[b]\n");
}

#[test]
fn issue1601_start_beyond_virtual_list_is_silent_empty() {
    let (stdout, stderr, code) = rubash("x=\"${@: -5}\"\necho \"[$x]\"", &["a", "b"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[]\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1601_star_form_uses_same_virtual_list() {
    let (stdout, _, code) = rubash("x=\"${*: -3:2}\"\necho \"[$x]\"", &["a", "b"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[probe a]\n");
}

#[test]
fn issue1601_slice_inside_larger_word() {
    let (stdout, _, code) = rubash("echo \"<${@: -2:1}>\"", &["a", "b"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<a>\n");
}
