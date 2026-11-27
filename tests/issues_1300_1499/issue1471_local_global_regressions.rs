//! rubash#1471 — `local -g` inside a function did not create a
//! global (the variable was a plain local and died with the call);
//! `declare -g` in the same position worked. GNU: `local -g` makes
//! the variable global, visible after the function returns.
//!
//! Probe: clusters/x2/repro/1471.sh.
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
fn local_g_creates_a_global() {
    let (stdout, stderr, code) = rubash(
        "f() { local -g G2=two; declare -g G1=one; }\n\
         f; echo \"G1=[${G1-unset}] G2=[${G2-unset}]\"\n\
         declare -p G2 2>&1\n\
         declare -p G1 2>&1\n\
         g() { local -g G3; G3=three; local plain=inner; }\n\
         g; echo \"G3=[${G3-unset}] plain=[${plain-unset}]\"",
    );
    assert_eq!(
        stdout,
        "G1=[one] G2=[two]\ndeclare -- G2=\"two\"\ndeclare -- G1=\"one\"\nG3=[three] plain=[unset]\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
