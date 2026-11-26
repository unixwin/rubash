//! rubash#1416 — an escaped `\~` was still tilde-expanded; GNU keeps
//! it literal (in word position and in assignment values, with or
//! without a user name following).
//!
//! Probe: clusters/k9/probes/p1416.sh.
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
fn escaped_tilde_stays_literal() {
    let (stdout, stderr, code) =
        rubash("echo \\~/x\nv=\\~/x; echo \"v=$v\"\necho \\~\necho \\~root");
    assert_eq!(stdout, "~/x\nv=~/x\n~\n~root\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
