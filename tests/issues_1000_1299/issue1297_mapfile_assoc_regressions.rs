//! rubash#1297 — `mapfile` into an associative array silently stored
//! garbage instead of erroring; GNU reports
//! "mapfile: h: not an indexed array" and returns 1, leaving h empty.
//!
//! Probe: clusters/k5/repro/r1297.sh.
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
fn mapfile_into_assoc_array_errors() {
    let (stdout, stderr, code) = rubash(
        "declare -A h\nprintf 'a\\n' | { mapfile h; echo \"rc=$? n=${#h[@]}\"; declare -p h; }",
    );
    assert_eq!(stdout, "rc=1 n=0\ndeclare -A h\n");
    assert!(
        stderr.contains("mapfile: h: not an indexed array"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
