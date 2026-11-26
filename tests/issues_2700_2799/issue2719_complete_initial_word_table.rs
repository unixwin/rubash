//! rubash#2719 — `complete -F _f -I icmd` must register the spec
//! under the INITIALWORD pseudo-name ONLY: GNU complete_builtin
//! (builtins/complete.def:417-424, 480-485) replaces the operand
//! list with the single pseudo word (DEFAULTCMD / EMPTYCMD /
//! INITIALWORD) whenever -D/-E/-I is present, for registration,
//! -p and -r alike. rubash additionally registered every operand
//! in the regular table, so `complete -p icmd` printed the spec
//! where GNU errors "no completion specification" (probed on GNU
//! bash 5.3.20: reg rc=0, -p icmd rc=1, -r icmd rc=1, -p -I and
//! bare -p list show only `complete -F _f -I`).

use std::process::Command;

fn run(script: &str) -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

const REGISTER: &str = "_f2719() { COMPREPLY=(a); }; complete -F _f2719 -I icmd2719";

#[test]
fn initial_word_registration_is_not_a_regular_spec() {
    let (stdout, stderr) = run(&format!(
        "{REGISTER}; echo reg=$?; complete -p icmd2719; echo p=$?"
    ));
    assert!(stdout.contains("reg=0"), "registration succeeds: {stdout}");
    assert!(stdout.contains("p=1"), "-p on the operand fails: {stdout}");
    assert!(
        stderr.contains("complete: icmd2719: no completion specification"),
        "-p names the missing spec: {stderr}"
    );
    assert!(
        !stdout.contains("complete -F _f2719 icmd2719"),
        "the spec must not print as a regular icmd2719 spec: {stdout}"
    );
}

#[test]
fn initial_word_spec_prints_and_removes_via_the_flag() {
    let (stdout, _) = run(&format!(
        "{REGISTER}; complete -p -I; echo pI=$?; complete -p; \
         complete -r icmd2719; echo r=$?; complete -r -I; echo rI=$?; complete -p -I; echo pI2=$?"
    ));
    assert!(
        stdout.contains("complete -F _f2719 -I"),
        "-p -I prints the initial-word spec: {stdout}"
    );
    assert!(stdout.contains("pI=0"), "{stdout}");
    assert!(stdout.contains("r=1"), "-r on the operand fails: {stdout}");
    assert!(stdout.contains("rI=0"), "-r -I removes it: {stdout}");
    assert!(
        stdout.contains("pI2=1"),
        "after -r -I the pseudo spec is gone: {stdout}"
    );
    // The bare listing (the `complete -p` line above) shows the -I
    // spec once, in flag form, and never as an icmd2719 entry.
    assert!(
        !stdout.contains("icmd2719"),
        "no output line names the operand: {stdout}"
    );
}

#[test]
fn compopt_initial_word_ignores_operands() {
    // complete.def:862-911 compopt_builtin: the same wl ? wl : list
    // rule — with -I present the operand names are ignored and the
    // INITIALWORD spec is the target.
    let (stdout, _) = run(&format!(
        "{REGISTER}; compopt +o nospace -I icmd2719; echo co=$?; compopt -I; echo list=$?"
    ));
    assert!(
        stdout.contains("co=0"),
        "compopt -I <operand> operates on the pseudo spec: {stdout}"
    );
    assert!(stdout.contains("list=0"), "{stdout}");
    assert!(
        stdout.contains("_InitialWorD_") || stdout.contains("-I"),
        "the compopt report names the initial-word spec: {stdout}"
    );
}

#[test]
fn plain_registration_unaffected() {
    let (stdout, _) = run(
        "_f2719() { COMPREPLY=(a); }; complete -F _f2719 plaincmd2719; \
         complete -p plaincmd2719; echo p=$?",
    );
    assert!(
        stdout.contains("complete -F _f2719 plaincmd2719"),
        "control: a plain registration still lands in the regular table: {stdout}"
    );
    assert!(stdout.contains("p=0"), "{stdout}");
}
