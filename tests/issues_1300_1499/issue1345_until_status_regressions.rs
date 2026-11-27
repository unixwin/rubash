//! rubash#1345 — an `until` loop discarded the last body command's
//! status and always returned 0; GNU returns the last body command's
//! status (like `while`).
//!
//! Probe: clusters/x6/probes/p1345.sh.
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
fn until_returns_last_body_status() {
    let (stdout, stderr, code) = rubash(
        "x=; until [ -n \"$x\" ]; do x=1; false; done; echo \"rc=$?\"\n\
         i=0; until [ $i -ge 2 ]; do i=$((i+1)); false; done; echo \"rc2=$?\"\n\
         g() { x=; until [ -n \"$x\" ]; do x=1; false; done; }\n\
         g; echo \"rcg=$?\"\n\
         until true; do :; done; echo \"rc0=$?\"\n\
         j=0; while [ $j -lt 1 ]; do j=$((j+1)); false; done; echo \"rcw=$?\"",
    );
    assert_eq!(stdout, "rc=1\nrc2=1\nrcg=1\nrc0=0\nrcw=1\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
