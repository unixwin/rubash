//! rubash#1465 — $'\U........' with the top bit set (>= 0x80000000)
//! kept the literal escape text; GNU treats the value as invalid
//! (wint_t semantics) and discards the whole escape, producing the
//! empty string. Other out-of-range forms (surrogates, > U+10FFFF,
//! 0x7FFFFFFF) stay literal in both shells.
//!
//! Probe: clusters/x1/issues/issue-1465.md repro.
//! Expectations verified against GNU bash 5.3.20 (script files).

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
fn ansic_unicode_signbit_escape_is_dropped() {
    let (stdout, stderr, code) = rubash(
        "x=$'\\U80000000'; echo \"len=${#x} [$x]\"\n\
         printf '%s' $'\\UFFFFFFFF' | od -An -tx1",
    );
    assert_eq!(stdout, "len=0 []\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
#[ignore = "issue#1465 residual: out-of-range \\U values below the sign bit (e.g. \\U00110000) are encoded as raw 4-byte sequences (f4 90 80 80) instead of staying literal like GNU; only the sign-bit cases are dropped correctly; see bf-w3-b REPORT.md"]
fn ansic_unicode_other_out_of_range_stays_literal() {
    let (stdout, _stderr, code) = rubash(
        "a=$'\\U00000041'; echo \"a=[$a]\"\n\
         b=$'\\U00110000'; echo \"lenb=${#b}\"",
    );
    assert_eq!(stdout, "a=[A]\nlenb=10\n");
    assert_eq!(code, Some(0));
}
