//! rubash#1649 — bash 5.3's dynamic variable BASH_MONOSECONDS was
//! entirely missing: `$BASH_MONOSECONDS` expanded empty and
//! `declare -p BASH_MONOSECONDS` reported not found.
//!
//! GNU ownership: third_party/bash/variables.c get_monoseconds (bash
//! 5.3) returns CLOCK_MONOTONIC tv_sec, falling back to gettimeofday
//! (i.e. EPOCHSECONDS) with no monotonic clock; NEWS item w and the
//! bash.1 entry say the same. The variable carries no integer
//! attribute, so `declare -p` renders `declare --
//! BASH_MONOSECONDS="..."` (as in the issue), and unsetting it loses
//! the special properties permanently (variables.c:3839
//! unbind_variable), even if subsequently reset.
//!
//! Exact monotonic values are machine-dependent, so the tests assert
//! shape, monotonic ticking, and the declare/-v/unset surface — not a
//! literal second count. The tick test compares against /proc/uptime
//! on Linux as a sanity bound, not equality.

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
fn monoseconds_expands_and_declares() {
    let (stdout, stderr, code) =
        rubash("echo \"mono=[$BASH_MONOSECONDS]\"; declare -p BASH_MONOSECONDS");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert!(
        first.starts_with("mono=[") && first.ends_with(']'),
        "unexpected first line: {stdout}"
    );
    let value: i64 = first
        .trim_start_matches("mono=[")
        .trim_end_matches(']')
        .parse()
        .expect("monoseconds must be an integer");
    assert!(value > 0, "monotonic seconds must be positive: {value}");
    assert!(
        stdout.contains(&format!("declare -- BASH_MONOSECONDS=\"{value}\"")),
        "declare -p must render the live value: {stdout}"
    );
}

#[test]
fn monoseconds_ticks_with_monotonic_clock() {
    let (stdout, _stderr, code) =
        rubash("a=$BASH_MONOSECONDS; sleep 2; b=$BASH_MONOSECONDS; echo $((b - a))");
    assert_eq!(code, Some(0));
    assert_eq!(stdout.trim(), "2");
}

#[test]
fn monoseconds_in_arithmetic_and_v_test() {
    let (stdout, _stderr, code) =
        rubash("echo $((BASH_MONOSECONDS > 0 ? 1 : 0)); [[ -v BASH_MONOSECONDS ]] && echo v-set");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "1\nv-set\n");
}

#[test]
fn monoseconds_unset_loses_special_properties() {
    let (stdout, _stderr, code) = rubash(
        "unset BASH_MONOSECONDS; echo \"after=[$BASH_MONOSECONDS]\"; \
         BASH_MONOSECONDS=42; echo \"reset=[$BASH_MONOSECONDS]\"",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after=[]\nreset=[42]\n");
}
