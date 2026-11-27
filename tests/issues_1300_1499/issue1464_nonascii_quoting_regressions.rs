//! rubash#1464 — non-ASCII bytes were emitted RAW by ${v@Q},
//! printf %q and declare/export -p; in the C locale GNU escapes them
//! as $'\ooo' octal sequences, keeping the representation ASCII-safe
//! and re-readable. (Invalid bytes and control characters were
//! already escaped correctly; only valid non-ASCII text took the raw
//! shortcut.)
//!
//! Probe: clusters/x1/issues/issue-1464.md repro (LC_ALL=C, byte-form
//! inputs so the file itself stays ASCII).
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
fn nonascii_bytes_are_octal_escaped_in_quoting() {
    let (stdout, stderr, code) = rubash(
        "export LC_ALL=C\n\
         v=$'caf\\xc3\\xa9'; echo \"${v@Q}\"\n\
         printf '%q\\n' $'\\xc3\\xa9'\n\
         export v=$'\\xc3\\xa9\\xc3\\xa8'; export -p | grep '^declare -x v='; declare -p v",
    );
    assert_eq!(
        stdout,
        "$'caf\\303\\251'\n$'\\303\\251'\ndeclare -x v=$'\\303\\251\\303\\250'\ndeclare -x v=$'\\303\\251\\303\\250'\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
