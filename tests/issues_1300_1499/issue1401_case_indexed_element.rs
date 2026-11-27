//! Issue rubash#1401: `declare -u`/`-l` case conversion not applied to
//! later indexed element assignment.
//!
//! GNU applies the case attribute when an indexed array element is stored
//! (`declare -u a; a[0]=abc` → `ABC`). Rubash only applied it at compound
//! init and scalar assignment time.
//!
//! Fix: src/executor/array_assignment_exec.rs execute_array_element_assignment
//! now applies uppercase/lowercase conversion to the element value.
//!
//! Expected outputs verified against GNU bash.

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
fn declare_u_applies_to_indexed_element() {
    let (stdout, _, code) = rubash(r#"declare -u a; a[0]=abc; echo "i0=${a[0]}""#);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("i0=ABC"));
}

#[test]
fn declare_l_applies_to_indexed_element() {
    let (stdout, _, code) = rubash(r#"declare -l a; a[0]=ABC; echo "i0=${a[0]}""#);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("i0=abc"));
}
