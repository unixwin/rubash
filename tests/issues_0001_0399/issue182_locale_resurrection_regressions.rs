//! rubash#182 — Locale variable (LANG) "resurrected" / dead
//! assignment.
//!
//! GNU variables.c: a locale variable assigned in the shell behaves
//! like any other variable — reading it back yields the assigned
//! value. The filed symptom: a child shell spawned with `LANG` in its
//! environment saw an EMPTY `LANG` (the engine's startup locale
//! handling clobbered the imported value).
//!
//! The test spawns the binary with LANG=marker123 in the environment
//! (the filed form); the child must echo it back verbatim.
//! Expectations verified against GNU bash 5.3.20.

use std::process::Command;

#[test]
fn imported_lang_survives_startup() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("LANG", "marker123")
        .arg("-c")
        .arg("echo \"LANG=[$LANG]\"")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "LANG=[marker123]\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn lang_assignment_round_trips() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("LANG=abc123; echo \"LANG=[$LANG]\"")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "LANG=[abc123]\n");
    assert_eq!(output.status.code(), Some(0));
}
