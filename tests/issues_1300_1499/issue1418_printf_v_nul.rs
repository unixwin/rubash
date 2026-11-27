//! Issue rubash#1418: printf -v stores embedded NUL bytes (GNU truncates).
//!
//! GNU treats the -v value as a C string, truncating at the first NUL.
//!
//! Fix: src/builtins/printf.rs assign_printf_output truncates at first NUL.

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
fn printf_v_truncates_at_nul() {
    let (stdout, _, code) = rubash(r#"printf -v pv 'a\0b'; echo "len=${#pv}""#);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("len=1"));
}
