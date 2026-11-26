//! rubash#1486 — `umask 7777` stored 7777 verbatim; GNU masks the
//! value to 0777 (only the permission bits are kept), and `umask`
//! prints the masked value.
//!
//! Probe: clusters/x3/repro/1486.sh.
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
fn umask_value_is_masked_to_0777() {
    let (stdout, stderr, code) = rubash("umask 7777\numask\numask 027\numask");
    assert_eq!(stdout, "0777\n0027\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
