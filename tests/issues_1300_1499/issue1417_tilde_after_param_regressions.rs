//! rubash#1417 — a tilde produced BY parameter expansion was
//! re-expanded (`~$u` with unset u collapsed to $HOME); GNU never
//! re-scans expansion results for tildes: the `~` stays literal and
//! the expansion is simply concatenated.
//!
//! Probe: clusters/k9/probes/p1417.sh (HOME fixed in-script).
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
fn tilde_from_expansion_is_not_reexpanded() {
    let (stdout, stderr, code) = rubash(
        "HOME=/rubash-w3b-home\n\
         unset u; v=~$u; echo \"v=[$v]\"\n\
         u=/x; v=~$u; echo \"v=[$v]\"\n\
         u=$HOME; v=~$u; echo \"v=[$v]\"",
    );
    assert_eq!(stdout, "v=[~]\nv=[~/x]\nv=[~/rubash-w3b-home]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
