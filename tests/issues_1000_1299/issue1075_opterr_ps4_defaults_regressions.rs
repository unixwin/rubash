//! rubash#1075: GNU binds `OPTERR=1` and `PS4=+ ` at startup; both
//! were simply unset in rubash.
//!
//! GNU anchor: `variables.c` `initialize_shell_variables` sets the
//! getopts diagnostic default (OPTERR=1) and the xtrace prefix
//! (PS4="+ ") for every fresh shell, as ordinary variables that
//! introspection (`${OPTERR-unset}`) can see. Explicitly assigned
//! values were already respected by both shells.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env_remove("OPTERR")
        .env_remove("PS4")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The reported case, with the already-agreeing explicit-set
/// boundary appended.
#[test]
fn opterr_and_ps4_have_gnu_defaults() {
    let (stdout, _, code) = rubash(
        "echo \"[${OPTERR-unset}]\"; echo \"[${PS4-unset}]\"; OPTERR=0; echo \"[${OPTERR-unset}]\"",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[1]\n[+ ]\n[0]\n");
}
