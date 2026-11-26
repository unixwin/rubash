//! rubash#1286 — printf numeric overflow: the diagnostic wording and
//! the overflowed %(fmt)T epoch handling diverged from GNU (rubash
//! printed a different message; the epoch wrapped to a far-future year
//! instead of falling back to the clamped value's date).
//!
//! GNU: an out-of-range operand is diagnosed with
//! "Numerical result out of range", the clamped value is still printed,
//! and the builtin returns 1. Probe: clusters/k5/repro/r1286.sh.
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
fn printf_overflow_clamps_diagnoses_and_returns_1() {
    let (stdout, stderr, code) = rubash(
        "printf '%d\\n' 99999999999999999999; echo \"rc=$?\"\n\
         TZ=UTC printf '%(%F)T\\n' 99999999999999999999; echo \"rc=$?\"\n\
         TZ=UTC printf '%(%F)T\\n' -99999999999999999999; echo \"rc=$?\"",
    );
    assert_eq!(
        stdout,
        "9223372036854775807\nrc=1\n1970-01-01\nrc=1\n1970-01-01\nrc=1\n"
    );
    assert_eq!(
        stderr.matches("Numerical result out of range").count(),
        3,
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
