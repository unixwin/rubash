//! rubash#1457 — `read -a` overwrote a READONLY array (rc=0, no
//! diagnostic). GNU refuses: "roarr: readonly variable", rc=1, and
//! the array keeps its original elements. (Scalar `read` into a
//! readonly variable was already rejected.)
//!
//! Probe: clusters/x1/issues/issue-1457.md repro.
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
fn read_a_into_readonly_array_is_refused() {
    let (stdout, stderr, code) = rubash(
        "readonly roarr=(orig1 orig2)\n\
         printf 'new1 new2\\n' | { read -a roarr; echo \"rc=$? now=[${roarr[*]}]\"; }",
    );
    assert_eq!(stdout, "rc=1 now=[orig1 orig2]\n");
    assert!(
        stderr.contains("roarr: readonly variable"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
