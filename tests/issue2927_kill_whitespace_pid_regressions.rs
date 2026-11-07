//! rubash#2927: `kill` rejects pid operands with surrounding
//! whitespace; GNU parses them (strtol).
//!
//! GNU ground truth (bash 5.3.20): builtins/kill.def:194 routes the
//! operand through `valid_number` (general.c — skips leading
//! whitespace, an optional sign, digits, trailing whitespace, whole
//! word consumed), so `kill ' 12'`, `'12 '`, `'\t12'`, `' +12'`
//! all reach kill(2) and report ESRCH for a dead pid, while
//! `'+ 12'` (sign separated from the digits) stays invalid.
//!
//! The pins use pid 999999 (certainly dead on every host) instead of
//! the issue's literal 12, which may name a live process elsewhere.

use std::process::{Command, Stdio};

fn run_shell(body: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Whitespace-padded pids parse and reach kill(2): ESRCH for a dead
/// pid, rc 1, GNU's `(pid) - No such process` message.
#[test]
fn kill_parses_whitespace_padded_pids() {
    for operand in [" 999999", "999999 ", "\t999999", " +999999", "999999\t"] {
        let (stdout, stderr, rc) = run_shell(&format!("kill '{operand}'; echo rc=$?"));
        assert_eq!(rc, Some(0), "kill '{operand}': {stdout:?} {stderr:?}");
        assert!(
            stdout.contains("rc=1"),
            "kill '{operand}' should fail with rc 1: {stdout:?}"
        );
        assert!(
            stderr.contains("(999999) - No such process"),
            "kill '{operand}': expected ESRCH message: {stderr:?}"
        );
    }
}

/// A sign separated from the digits by whitespace stays invalid
/// (valid_number requires the digits to follow the sign at once).
#[test]
fn kill_rejects_sign_separated_from_digits() {
    let (_, stderr, rc) = run_shell("kill '+ 999999'; echo rc=$?");
    assert_eq!(rc, Some(0));
    assert!(
        stderr.contains("not a pid or valid job spec"),
        "kill '+ 999999': expected invalid-spec diagnostic: {stderr:?}"
    );
}

/// A leading zero parses as decimal (strtol base 10), and `0x10`
/// stays invalid — the strtol shape, not octal/hex parsing. Uses
/// the certainly-dead pid 999999 (never the literal 12, which may
/// name a live process on another host).
#[test]
fn kill_pid_number_forms_match_strtol() {
    let (_, stderr, rc) = run_shell("kill ' 0999999'; echo rc=$?");
    assert_eq!(rc, Some(0));
    assert!(
        stderr.contains("(999999) - No such process"),
        "kill ' 0999999' should parse as pid 999999: {stderr:?}"
    );
    let (_, stderr, _) = run_shell("kill '0x10'; echo rc=$?");
    assert!(
        stderr.contains("not a pid or valid job spec"),
        "kill '0x10' should stay invalid: {stderr:?}"
    );
}

/// A padded pid of a LIVE job still signals it (the parse must not
/// change delivery, only admission).
#[test]
fn kill_padded_live_pid_signals_the_job() {
    let (stdout, _, rc) = run_shell("sleep 3 & p=$!; kill \" $p\"; wait $p; echo w=$?");
    assert_eq!(rc, Some(0));
    assert!(
        stdout.contains("w=143"),
        "kill \" $p\" should deliver TERM (w=143): {stdout:?}"
    );
}
