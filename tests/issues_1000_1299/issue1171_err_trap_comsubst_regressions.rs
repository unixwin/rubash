//! rubash#1171 — ERR trap dropped for unrescued failing comsubst
//! under set -e.
//!
//! GNU trap.c / execute_cmd.c: when `x=$(false)` fails under `set -e`
//! and nothing rescues it, the ERR trap fires BEFORE the shell exits;
//! the following command never runs and the exit status is the
//! failed command's (1).
//!
//! Expectations byte-verified against GNU bash 5.3.20 (`-c` string).

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
fn err_trap_fires_for_failing_comsubst_under_errexit() {
    let (stdout, _, code) = rubash("set -e\ntrap \"echo TRAP\" ERR\nx=$(false)\necho AFTER");
    assert_eq!(stdout, "TRAP\n");
    assert_eq!(code, Some(1));
}
