//! rubash#1492 — `declare -p` showed no integer attribute for
//! HISTCMD / OPTIND (nor the readonly-integer form for UID/EUID/
//! PPID); GNU declares them `declare -i` / `declare -ir`.
//!
//! Probe: clusters/x4/repro/1492.sh. (PPID/UID values are
//! environment-specific; the attribute text is the pinned part.)
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
fn declare_p_shows_integer_attributes_for_special_vars() {
    let (stdout, _stderr, code) =
        rubash("declare -p HISTCMD OPTIND\ndeclare -p UID EUID PPID 2>&1 | head -3");
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some("declare -i HISTCMD=\"0\""),
        "stdout: {stdout}"
    );
    assert_eq!(
        lines.next(),
        Some("declare -i OPTIND=\"1\""),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("declare -ir UID=\""), "stdout: {stdout}");
    assert!(stdout.contains("declare -ir EUID=\""), "stdout: {stdout}");
    assert!(stdout.contains("declare -ir PPID=\""), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}
