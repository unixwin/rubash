//! Issue rubash#1799 regression: the `compgen -A variable` candidate
//! set differed from GNU in three names — PIPESTATUS was always
//! listed, HISTCMD never, and the engine-injected THIS_SH leaked in.
//!
//! GNU pcomplete.c it_init_variables lists all_visible_variables()
//! (variables.c:4319): every non-invisible cell in the variable
//! table. HISTCMD is registered by initialize_dynamic_variables
//! (variables.c:1873 INIT_DYNAMIC_VAR) and always lists. PIPESTATUS
//! has no cell at startup; set_pipestatus_array (variables.c:6291)
//! materializes it when the first pipeline completes (any completed
//! command does — a simple command is a one-stage pipeline), and a
//! scalar bind creates a visible scalar cell, while valueless
//! declare/readonly/export shapes stay att_invisible. THIS_SH is not
//! a GNU variable at all; an imported THIS_SH is an ordinary
//! environment entry and lists.
//!
//! Oracle: GNU bash 5.3.20 — fresh `compgen -A variable` contains
//! HISTCMD, not PIPESTATUS; after `true`, PIPESTATUS appears;
//! `THIS_SH=/bin/zsh compgen -A variable` (imported) lists THIS_SH.

use std::process::Command;

fn rubash(script: &str) -> String {
    rubash_env(script, &[])
}

fn rubash_env(script: &str, env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c").arg(script);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let output = cmd.output().expect("run rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn listed(script: &str, name: &str) -> bool {
    rubash(script).lines().any(|line| line == name)
}

#[test]
fn fresh_shell_lists_histcmd_but_not_pipestatus_or_this_sh() {
    // compgen -A variable must be the FIRST command: any completed
    // command materializes PIPESTATUS (see the next test).
    let stdout = rubash("compgen -A variable");
    assert!(
        stdout.lines().any(|line| line == "HISTCMD"),
        "HISTCMD is a registered dynamic variable and must list: {stdout}"
    );
    assert!(
        !stdout.lines().any(|line| line == "PIPESTATUS"),
        "no PIPESTATUS cell exists before the first pipeline completes"
    );
    assert!(
        !stdout.lines().any(|line| line == "THIS_SH"),
        "the engine-injected THIS_SH is not a shell variable"
    );
}

#[test]
fn pipestatus_lists_once_a_command_has_completed() {
    assert!(listed("true; compgen -A variable", "PIPESTATUS"));
    assert!(listed("true | true; compgen -A variable", "PIPESTATUS"));
    // A scalar bind creates a visible scalar cell even though the
    // status writer leaves it frozen.
    assert!(listed("PIPESTATUS=99; compgen -A variable", "PIPESTATUS"));
    // Valueless declare shapes are att_invisible in GNU: no listing.
    assert!(!listed(
        "declare PIPESTATUS; compgen -A variable",
        "PIPESTATUS"
    ));
}

#[test]
fn pipestatus_arrayvar_listing_follows_the_live_array_cell() {
    assert!(!listed("compgen -A arrayvar", "PIPESTATUS"));
    assert!(listed("true; compgen -A arrayvar", "PIPESTATUS"));
    // A scalar PIPESTATUS cell is a variable, but not an arrayvar.
    assert!(!listed("PIPESTATUS=99; compgen -A arrayvar", "PIPESTATUS"));
}

#[test]
fn caller_supplied_this_sh_wrapper_still_lists() {
    // Init keeps an inherited THIS_SH that names an existing sh/bash
    // wrapper file; that value is a genuine environment variable (an
    // imported THIS_SH lists under GNU), unlike the injected spelling.
    let dir = std::env::temp_dir().join(format!("rubash-1799-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let wrapper = dir.join("bash");
    std::fs::write(&wrapper, "#!/bin/sh\nexit 0\n").expect("wrapper file");
    let wrapper_text = wrapper.to_string_lossy().into_owned();
    let stdout = rubash_env("compgen -A variable", &[("THIS_SH", &wrapper_text)]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        stdout.lines().any(|line| line == "THIS_SH"),
        "a caller-supplied THIS_SH wrapper must list: {stdout}"
    );
}
