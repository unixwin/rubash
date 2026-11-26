//! rubash#1472 — `umask -S` / `umask -p` printed the plain octal mask
//! on direct invocation, ignoring the requested print form. Closed in
//! the zero-queue campaign on x2-cluster evidence (byte-identical to
//! GNU bash 5.3.20 across direct `-S`/`-p`/`-pS`, command-substitution
//! and the 0000/0022/0027 masks). GNU source: builtins/umask.def —
//! the print branch honours the symbolic / re-inputable forms.
//!
//! The script sets the mask before every print, so the expectations
//! do not depend on the umask the test process inherits.

use std::process::Command;

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1472_umask_print_forms_match_gnu() {
    let script = "umask 0022; umask; umask -S; umask -p; umask -pS; x=$(umask -S); echo \"comsub=$x\"; umask 0000; umask -S; umask -p; umask 0027; umask -S; umask -p; umask";
    let (stdout, stderr, code) = rubash_c(script);
    assert_eq!(
        stdout,
        "0022\nu=rwx,g=rx,o=rx\numask 0022\numask -S u=rwx,g=rx,o=rx\ncomsub=u=rwx,g=rx,o=rx\nu=rwx,g=rwx,o=rwx\numask 0000\nu=rwx,g=rx,o=\numask 0027\n0027\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
