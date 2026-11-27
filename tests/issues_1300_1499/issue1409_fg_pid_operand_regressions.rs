//! rubash#1409 — `fg <PID>` accepted a bare PID operand; GNU rejects
//! it like `bg` (rubash#1408): warning "job specification requires
//! leading `%'" then "no such job", rc=1.
//!
//! Probe: clusters/k9/probes/p1409.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn fg_bare_pid_is_rejected() {
    let (stdout, _stderr, code) = rubash(
        "set -m\nsleep 0.2 & p=$!\nfg $p 2>&1; echo \"rc=$?\"\nkill $p 2>/dev/null; wait 2>/dev/null; true",
    );
    assert!(stdout.contains("fg: warning:"), "stdout: {stdout}");
    assert!(
        stdout.contains("job specification requires leading `%'"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("no such job"), "stdout: {stdout}");
    assert!(stdout.contains("rc=1"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}
