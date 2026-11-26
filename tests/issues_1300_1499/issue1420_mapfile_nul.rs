//! Issue rubash#1420: mapfile keeps embedded NUL bytes (GNU truncates).
//!
//! GNU mapfile.def treats input as C-string; values truncate at first NUL.
//!
//! Fix: src/executor/arrays/mapfile.rs truncates at first NUL.

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
fn mapfile_truncates_at_nul() {
    let (stdout, _, code) =
        rubash(r#"printf 'a\0b\n' | { mapfile -t arr; echo "len=${#arr[0]}"; }"#);
    assert_eq!(code, Some(0));
    // 'a\0b' truncates to 'a' (len=1), not 'ab' (len=2).
    assert!(stdout.contains("len=1"));
}
