//! rubash#1233 — `~user` for a user that does not exist must stay
//! literal. The report environment (empty user database) showed an
//! invented fallback (`~root` -> `/`, `~u0_a123` -> `/data`). The
//! zero-queue campaign's x6 re-check on a glibc baseline established
//! the engine consults the real user database: known names resolve
//! via getpwnam exactly like GNU bash 5.3.20, and unknown names —
//! including the filed `u0_a*` shape — stay literal in word,
//! assignment and declare positions (subst.c tilde expansion leaves
//! the word unchanged when the lookup fails).
//!
//! The username used here (`rubashnosuchuser`) is chosen to not exist
//! on any realistic test host.

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
fn issue1233_unknown_user_tilde_stays_literal() {
    let script = "echo ~rubashnosuchuser\necho ~rubashnosuchuser/x\nv=~rubashnosuchuser; echo \"v=$v\"\nv2=~rubashnosuchuser/x; echo \"v2=$v2\"\ndeclare dr=~rubashnosuchuser; echo \"dr=$dr\"\necho ~u0_a123\n";
    let (stdout, stderr, code) = rubash_c(script);
    assert_eq!(
        stdout,
        "~rubashnosuchuser\n~rubashnosuchuser/x\nv=~rubashnosuchuser\nv2=~rubashnosuchuser/x\ndr=~rubashnosuchuser\n~u0_a123\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1233_known_user_tilde_resolves_via_user_database() {
    // Positive control: `root` exists in every realistic /etc/passwd
    // with home /root; GNU resolves it through getpwnam.
    let (stdout, stderr, code) = rubash_c("echo ~root");
    assert_eq!(stdout, "/root\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
