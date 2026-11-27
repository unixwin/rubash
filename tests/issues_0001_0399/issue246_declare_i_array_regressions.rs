//! rubash#246 — `declare -i` on an array name converts its elements
//! to integers.
//!
//! GNU builtins/declare.def / variables.c: declaring an existing
//! indexed array with -i applies the integer attribute to the array:
//! element values are re-evaluated as arithmetic (`"3"` stays 3), the
//! array keeps its elements, and `declare -p` reports the -ai
//! attribute pair.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (`-c` string).

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
fn declare_i_converts_array_elements() {
    let (stdout, _, code) =
        rubash("x=(3); declare -i x; declare -p x; echo \"elem0=${x[0]} n=${#x[@]}\"");
    assert_eq!(stdout, "declare -ai x=([0]=\"3\")\nelem0=3 n=1\n");
    assert_eq!(code, Some(0));
}
