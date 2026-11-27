//! Issue rubash#1395: Bare `export -a`/`-A` ignores the type filter
//! (lists all exported vars).
//!
//! GNU's `export -A` (no operands) lists only exported assoc arrays;
//! `export -a` lists only exported indexed arrays. Rubash listed all
//! exported variables.
//!
//! Fix: src/builtins/setattr.rs export_with_io now passes the array/assoc
//! flags to print_exported; src/builtins/setattr/print.rs filters by type.
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
fn bare_export_capital_a_filters_assoc() {
    let (stdout, _, code) = rubash(
        r#"v1=1; export v1
declare -A h=([k]=v); export -A h
export -A | wc -l"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "1");
}

#[test]
fn bare_export_lowercase_a_filters_indexed() {
    let (stdout, _, code) = rubash(
        r#"v1=1; export v1
declare -A h=([k]=v); export -A h
export -a | wc -l"#,
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "0");
}
