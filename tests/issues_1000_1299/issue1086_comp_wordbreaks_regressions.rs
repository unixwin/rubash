//! rubash#1086: `COMP_WORDBREAKS` is bound at startup to GNU's
//! default break set; rubash left it unset (`declare -p` reported
//! "not found") although assigning it worked.
//!
//! GNU anchor: `bashline.c` initializes COMP_WORDBREAKS to
//! ` \t\n"'@><=;|&(:)` for completion; `declare -p` renders it in
//! `$'...'` form. The value is part of the environment contract
//! that completion systems (and user rc files) read.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env_remove("COMP_WORDBREAKS")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The reported case: exact default bytes, rc 0.
#[test]
fn comp_wordbreaks_default_is_bound() {
    let (stdout, _, code) = rubash("declare -p COMP_WORDBREAKS; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declare -- COMP_WORDBREAKS=$' \\t\\n\"\\'@><=;|&(:'\nrc=0\n"
    );
}

/// Boundary that already agreed: an explicit assignment replaces
/// the default.
#[test]
fn comp_wordbreaks_assignment_still_works() {
    let (stdout, _, _) = rubash("COMP_WORDBREAKS=\"xyz\"; declare -p COMP_WORDBREAKS");
    assert_eq!(stdout, "declare -- COMP_WORDBREAKS=\"xyz\"\n");
}
