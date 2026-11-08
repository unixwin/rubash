//! rubash#2975 — bare `unalias` (no operands) exited 1; GNU exits 2.
//!
//! GNU mechanism (builtins/alias.def, unalias_builtin): with no
//! arguments the builtin calls sh_usage and returns EX_USAGE (2) —
//! the same status the invalid-option path already returns. Only a
//! genuinely missing alias name is EXECUTION_FAILURE (1).
//!
//! Expectations measured against GNU bash 5.3.20 (oracle binary),
//! script probes 2026-11-07: `unalias` prints
//! `unalias: usage: unalias [-a] name [name ...]` and exits 2.

use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

fn run_case(script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(["-c", script])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let out = wait_bounded(&mut child, Duration::from_secs(20));
    assert!(!out.timed_out, "case timed out: {script}");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.code,
    )
}

#[test]
fn issue2975_bare_unalias_exits_2_with_usage() {
    let (stdout, stderr, code) = run_case("unalias; echo \"rc=$?\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=2\n");
    assert_eq!(
        stderr.trim_end(),
        "unalias: usage: unalias [-a] name [name ...]"
    );
}

#[test]
fn issue2975_unalias_dashdash_only_exits_2() {
    let (stdout, _, _) = run_case("unalias --; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=2\n");
}

#[test]
fn issue2975_existing_and_missing_names_keep_their_statuses() {
    let (stdout, _, _) = run_case("alias q=w; unalias q; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=0\n");
    let (stdout, _, _) = run_case("unalias no_such_alias_xyz 2>/dev/null; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    let (stdout, _, _) = run_case("unalias -a; echo \"rc=$?\"");
    assert_eq!(stdout, "rc=0\n");
}
