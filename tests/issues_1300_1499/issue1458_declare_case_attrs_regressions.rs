//! rubash#1458 — `declare -l -u` on one variable kept the LAST case
//! attribute and converted values forever; GNU treats att_lowercase
//! and att_uppercase as independent bits where setting one clears
//! the other, so `-lu` together cancel out entirely (no conversion,
//! `declare -p` shows no case attribute).
//!
//! Probe: clusters/x1/issues/issue-1458.md repro.
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
fn declare_lower_upper_together_cancel() {
    let (stdout, stderr, code) = rubash(
        "declare -lu a=\"AbC\"; echo \"[$a]\"; declare -p a\n\
         declare -lu e=\"AbC\"; e=\"XyZ\"; echo \"[$e]\"",
    );
    assert_eq!(stdout, "[AbC]\ndeclare -- a=\"AbC\"\n[XyZ]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
