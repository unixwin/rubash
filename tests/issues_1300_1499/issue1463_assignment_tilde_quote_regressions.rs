//! rubash#1463 — any quote character in an assignment VALUE disabled
//! tilde expansion for the whole value; GNU tests per segment: a `~`
//! at the value start or right after a `:` expands even when another
//! segment is quoted.
//!
//! Probe: clusters/x1/issues/issue-1463.md repro (HOME fixed in-script
//! for determinism; the probe's <HOME> is the same value).
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
fn quoted_segment_does_not_disable_assignment_tilde() {
    let (stdout, stderr, code) = rubash(
        "HOME=/rubash-w3b-home\n\
         a=~/x:'q'; echo \"a=$a\"\n\
         b='q':~/x; echo \"b=$b\"\n\
         d=~/x:~/y; echo \"d=$d\"",
    );
    assert_eq!(
        stdout,
        "a=/rubash-w3b-home/x:q\nb=q:/rubash-w3b-home/x\nd=/rubash-w3b-home/x:/rubash-w3b-home/y\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
