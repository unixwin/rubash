//! rubash#1287 — `${#$'...'}` (length of an ANSI-C quoted string) was a
//! bad substitution; GNU expands it to the string length with rc=0.
//!
//! Probe: clusters/k5/repro/r1287.sh.
//! Expectations byte-verified against GNU bash 5.3.20 (script files).

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
fn length_of_ansic_quote_expands() {
    // GNU quirk, pinned as measured: ${#$'abc'} is 0, not 3 — the
    // length is taken of the (empty) parameter named by the quote.
    let (stdout, stderr, code) = rubash("echo \"${#$'abc'}\"\necho \"rc=$?\"");
    assert_eq!(stdout, "0\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
