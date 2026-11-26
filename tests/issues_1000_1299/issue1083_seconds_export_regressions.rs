//! rubash#1083 — SECONDS negative start clamped+frozen (plus
//! export -pf ordering).
//!
//! GNU variables.c: assigning a negative value to SECONDS sets the
//! epoch offset so the variable reads back the assigned value and then
//! TICKS UP from there (-100, -99, ...). It is neither clamped to 0 nor
//! frozen. Separately, `export -pf` prints the function definition
//! followed by its `declare -fx` line.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings); the
//! SECONDS read-backs allow one second of scheduling slack, the export
//! output is byte-exact.

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
fn seconds_negative_start_reads_back_and_ticks_up() {
    let (stdout, _, code) = rubash("SECONDS=-100; echo $SECONDS; sleep 1.2; echo $SECONDS");
    let values: Vec<i64> = stdout
        .lines()
        .map(|line| line.trim().parse().expect("SECONDS value is numeric"))
        .collect();
    assert_eq!(values.len(), 2, "stdout: {stdout}");
    assert_eq!(values[0], -100, "assigned value reads back verbatim");
    assert!(
        values[1] > values[0] && values[1] <= -98,
        "SECONDS must tick up from -100 (not clamped, not frozen): {values:?}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn export_pf_prints_definition_then_declare_line() {
    let (stdout, _, code) = rubash("a1(){ echo one; }; export -f a1; export -pf");
    assert_eq!(stdout, "a1 () \n{ \n    echo one\n}\ndeclare -fx a1\n");
    assert_eq!(code, Some(0));
}
