//! Issue rubash#1618: compound append `a+=(...)` / `A+=(...)` skipped
//! the case attributes, and an uppercase assoc append lost the value
//! (the whole storage string was case-folded, folding the KEYS).
//!
//! GNU assign_compound_array_list (arrayfunc.c:700) binds each element
//! through bind_array_variable / bind_assoc_variable, so every appended
//! element value runs through make_variable_value (variables.c:2905) —
//! case conversion applies to element VALUES only, never to assoc keys.
//!
//! Expected outputs verified against GNU bash 5.3.20.

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
fn issue1618_indexed_compound_append_lowercase() {
    let (stdout, _, code) =
        rubash(r#"declare -l a=(x); a+=(NewOne); echo "lower:${a[0]} ${a[1]}""#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "lower:x newone\n");
}

#[test]
fn issue1618_indexed_compound_append_uppercase() {
    let (stdout, _, _) = rubash(r#"declare -u a=(x); a+=(NewOne); echo "upper:${a[0]} ${a[1]}""#);
    assert_eq!(stdout, "upper:X NEWONE\n");
}

#[test]
fn issue1618_assoc_compound_append_value_not_key() {
    let (stdout, _, _) =
        rubash(r#"declare -uA A=([k]=x); A+=([new]="New"); echo "assoc:${A[new]}""#);
    assert_eq!(stdout, "assoc:NEW\n");
    // The key must NOT be folded: reading with the written key works,
    // and no uppercased key materializes.
    let (stdout, _, _) = rubash(r#"declare -uA A=([k]=x); A+=([new]="New"); echo "keys:${!A[@]}""#);
    assert!(stdout.contains("new"), "keys folded: {stdout}");
    assert!(!stdout.contains("NEW"), "keys folded: {stdout}");
}

#[test]
fn issue1618_assoc_compound_init_converts_values_not_keys() {
    let (stdout, _, _) = rubash(r#"declare -uA A; A=([k]=v [n]=w); echo "vals:${A[k]} ${A[n]}""#);
    assert_eq!(stdout, "vals:V W\n");
}

#[test]
fn issue1618_indexed_compound_init_converts_new_elements() {
    let (stdout, _, _) = rubash(r#"declare -l la; la=(X Y); echo "init:${la[0]} ${la[1]}""#);
    assert_eq!(stdout, "init:x y\n");
}

#[test]
fn issue1618_scalar_append_case_transforms_whole_result() {
    let (stdout, _, _) = rubash(r#"declare -u s=abc; s+=def; echo "scalarapp:$s""#);
    assert_eq!(stdout, "scalarapp:ABCDEF\n");
}

#[test]
fn issue1618_element_append_case_transforms_post_append() {
    let (stdout, _, _) = rubash(r#"declare -la a=(AB); a[0]+=cd; echo "elemapp:${a[0]}""#);
    assert_eq!(stdout, "elemapp:abcd\n");
}
