//! Issue rubash#1366: `declare -p` omits the `-r` flag after
//! `readonly GROUPS` / `readonly LINENO` (display-only).
//!
//! The readonly attribute is enforced (unset fails), but `declare -p`
//! did not display the `-r` flag for these dynamic special variables.
//!
//! Fix: src/executor/dynamic_arrays.rs dynamic_declaration_view now
//! includes the readonly flag in the output.
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
fn declare_p_shows_readonly_for_groups() {
    let (stdout, _, code) = rubash(r#"readonly GROUPS; declare -p GROUPS"#);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("declare -ar GROUPS="));
}

#[test]
fn declare_p_shows_readonly_for_lineno() {
    let (stdout, _, code) = rubash(r#"readonly LINENO; declare -p LINENO"#);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("declare -r LINENO="));
}
