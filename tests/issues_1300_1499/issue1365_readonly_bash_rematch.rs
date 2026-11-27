//! Issue rubash#1365: `=~` silently overwrites a readonly BASH_REMATCH;
//! GNU errors and keeps the stale value.
//!
//! GNU variables.c bind_variable refuses the store when the variable is
//! readonly, emitting "BASH_REMATCH: readonly variable" on stderr. The
//! match itself still succeeds (rc=0); only the capture store is refused.
//!
//! Fix: src/executor/conditional.rs store_bash_rematch checks the readonly
//! mark before overwriting.
//!
//! Expected outputs verified against GNU bash 5.3 behavior per the issue.

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
fn readonly_bash_rematch_keeps_stale_value() {
    let (stdout, stderr, code) = rubash(
        r#"[[ xyz =~ (y) ]]
readonly BASH_REMATCH
[[ abc =~ (b) ]]; echo "m=$? m0=${BASH_REMATCH[0]-U}""#,
    );
    assert_eq!(code, Some(0));
    // Match succeeds (m=0), but capture store refused, stale value kept.
    assert!(stdout.contains("m=0 m0=y"));
    assert!(stderr.contains("BASH_REMATCH: readonly variable"));
}

#[test]
fn readonly_bash_rematch_no_prior_value() {
    let (stdout, stderr, code) = rubash(
        r#"readonly BASH_REMATCH
[[ abc =~ (b) ]]; echo "m=$? m0=${BASH_REMATCH[0]-U}""#,
    );
    assert_eq!(code, Some(0));
    assert!(stdout.contains("m=0 m0=U"));
    assert!(stderr.contains("BASH_REMATCH: readonly variable"));
}
