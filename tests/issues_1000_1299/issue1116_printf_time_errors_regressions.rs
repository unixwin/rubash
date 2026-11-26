//! rubash#1116 — printf %T error paths diverge (bare %T prints time;
//! invalid arg rc=0).
//!
//! GNU builtins/printf.def: a bare `%T` with no argument is an error
//! (`printf: `T': invalid format character`, rc=1, no output); a
//! non-numeric time argument and an empty time argument are
//! `invalid number` errors with rc=1 (the format's literal text is
//! still produced for the parts that do not depend on the value).
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings): error
//! texts matched on their stable core (line prefix carries the
//! binary's own path); stdout and rc are byte-/value-exact.

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
fn bare_percent_t_is_an_error() {
    let (stdout, stderr, code) = rubash("printf '%T\\n'; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("printf: `T': invalid format character"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn invalid_time_arguments_are_errors() {
    let (stdout, stderr, code) = rubash(
        "printf '%(d)T\\n' garbage; echo \"rc=$?\"; printf '%(%Y)T\\n' \"\"; echo \"rc=$?\"",
    );
    assert_eq!(stdout, "d\nrc=1\n1970\nrc=1\n");
    assert!(
        stderr.contains("printf: garbage: invalid number"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("printf: : invalid number"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
