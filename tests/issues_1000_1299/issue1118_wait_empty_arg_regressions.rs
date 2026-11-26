//! rubash#1118 — `wait ""` message differs and rc=127 (GNU rc=1).
//!
//! GNU builtins/wait.def: an empty id is "not a pid or valid job spec"
//! and the builtin returns 1; 127 would falsely imply command-not-found.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` string): the
//! message is matched on its stable core (line prefix carries the
//! binary's own path), rc is exact.

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
fn wait_empty_string_is_not_a_job_spec() {
    let (stdout, stderr, code) = rubash("wait \"\"; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("wait: `': not a pid or valid job spec"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
