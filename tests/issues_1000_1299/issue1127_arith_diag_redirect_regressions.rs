//! rubash#1127 — `(( ))` arithmetic error diagnostic ignores the
//! command's own `2>&1` redirection.
//!
//! GNU: a diagnostic produced while executing a command goes through
//! the command's redirections like any other output of that command.
//! With `2>&1` attached to the `(( ))`, the "value too great for base"
//! diagnostic lands on stdout and the shell's own stderr stays empty.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` string): stdout
//! carries the diagnostic text followed by the echo line; matched on
//! the stable core (line prefix carries the binary's own path).

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
fn arith_diagnostic_honors_command_redirection() {
    let (stdout, stderr, code) = rubash("a=1x; ((a+=1)) 2>&1; echo \"a=$a rc=$?\"");
    assert!(
        stdout.contains("((: 1x: value too great for base (error token is \"1x\")"),
        "stdout: {stdout}"
    );
    assert!(stdout.ends_with("a=1x rc=1\n"), "stdout: {stdout}");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
