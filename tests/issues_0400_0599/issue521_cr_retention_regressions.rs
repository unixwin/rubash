//! niubash#521 (engine-side GNU pin) — Carriage returns in input
//! are DATA, not line-ending decoration.
//!
//! The niu issue asks whether dropping a trailing CR is correct.
//! GNU's answer, pinned here on the engine: `read` and command
//! substitution keep a trailing `\r` (only `\n` terminates/strips);
//! CR stripping is a deliberate niu-side Windows behavior layered
//! above the engine, not engine semantics. `read -d ''` (NUL
//! delimiter) keeps the whole record, `\r` and `\n` alike.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (`-c` string):
//! for input `a\r\n`, the captured lengths are 2 / 2 / 3.

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
fn trailing_cr_is_retained_by_read_and_comsub() {
    let (stdout, _, code) = rubash(
        "v=$(printf 'a\\r\\n'); echo \"comsub=${#v}\"\n\
         read x < <(printf 'a\\r\\n'); echo \"read=${#x}\"\n\
         IFS= read -d '' y < <(printf 'a\\r\\n'); echo \"readd=${#y}\"\n\
         printf 'a\\r\\n' | { read z; echo \"pipe=${#z}\"; }",
    );
    assert_eq!(stdout, "comsub=2\nread=2\nreadd=3\npipe=2\n");
    assert_eq!(code, Some(0));
}
