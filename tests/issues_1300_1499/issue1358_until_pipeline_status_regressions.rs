//! rubash#1358 — an `until` loop's exit status ignored a failing
//! PIPELINE as the last body command (always 0); `while`/`for` already
//! agreed with GNU. Sibling of rubash#1345 (plain-command form).
//!
//! Probe: clusters/z4/probes/final/1358.sh (+ gnu-1358.out).
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
fn until_returns_failing_pipeline_status() {
    let (stdout, stderr, code) = rubash(
        "x=; until [ -n \"$x\" ]; do x=1; true | false; done; echo \"rc=$?\"\n\
         i=0; until [ $i -ge 2 ]; do i=$((i+1)); true | false; done; echo \"rc2=$?\"\n\
         x=; until [ -n \"$x\" ]; do x=1; false; done; echo \"rc3=$?\"\n\
         i=0; while [ $i -lt 1 ]; do i=$((i+1)); true | false; done; echo \"while_rc=$?\"\n\
         for i in 1 2; do true | false; done; echo \"for_rc=$?\"\n\
         x=; until [ -n \"$x\" ]; do x=1; true; done; echo \"ok_rc=$?\"",
    );
    assert_eq!(
        stdout,
        "rc=1\nrc2=1\nrc3=1\nwhile_rc=1\nfor_rc=1\nok_rc=0\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
