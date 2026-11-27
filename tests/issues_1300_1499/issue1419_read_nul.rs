//! Issue rubash#1419: bare read keeps embedded NUL bytes (GNU strips them).
//!
//! GNU read.def treats input as C string; NULs are stripped.
//!
//! Fix: src/executor/read_builtin.rs strips NULs from input line
//! (except when delimiter is NUL, where NUL terminates).

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
fn read_strips_nul_bytes() {
    // Use printf to generate NUL via shell
    let (stdout, _, code) = rubash(r#"printf 'a\0b\n' | { read -r v; echo "len=${#v}"; }"#);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("len=2"));
}
