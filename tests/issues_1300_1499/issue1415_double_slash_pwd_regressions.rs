//! rubash#1415 — in a `//`-prefixed working directory (UNC form on
//! Windows, POSIX double-slash on Linux), $PWD / `pwd -L` dropped one
//! leading slash, making $PWD unresolvable (`cd "$PWD"` failed);
//! `pwd -P` was correct. GNU preserves the double slash in the
//! logical path (POSIX permits the // prefix to be special).
//!
//! Probe: clusters/k9/probes/p1415.sh.
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
fn double_slash_prefix_survives_in_logical_pwd() {
    let (stdout, stderr, code) =
        rubash("cd //tmp; echo \"PWD=$PWD\"; pwd -L; pwd -P; cd \"$PWD\" && echo cdPWD-ok");
    assert_eq!(stdout, "PWD=//tmp\n//tmp\n//tmp\ncdPWD-ok\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
