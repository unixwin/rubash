//! rubash#3820 — `getopts` mishandles `:` and `?`:
//!
//! 1. `:` is never a valid option letter in GNU bash (`lib/sh/getopt.c`),
//!    even when it appears in optstring after the leading silent-mode `:`
//!    or after another option letter (`::a`, `:a:`, `a:`, `::`, `a:b:`).
//! 2. When `?` IS declared as an option letter in optstring, `sh_getopt`
//!    returns `'?'` without emitting an illegal-option diagnostic, and
//!    `dogetopts` (`builtins/getopts.def`) enters the `ret == '?'` branch:
//!    silent mode binds `OPTARG` to `"?"`, while noisy mode unbinds `OPTARG`.
//!
//! Expectations verified against GNU bash 5.3.20.

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
fn issue3820_colon_is_never_a_valid_option_letter_silent() {
    for (label, optstring) in [
        ("colon-after-silent", "::a"),
        ("colon-after-letter", ":a:"),
        ("colon-only", "::"),
        ("colon-control", ":a"),
    ] {
        let script = format!("getopts '{optstring}' o -:; echo \"o=$o OPTARG=[$OPTARG]\"");
        let (stdout, stderr, code) = run_case(&script);
        assert_eq!(code, Some(0), "{label}: unexpected exit code");
        assert_eq!(stdout, "o=? OPTARG=[:]\n", "{label}: stdout mismatch");
        assert!(stderr.is_empty(), "{label}: unexpected stderr: {stderr}");
    }
}

#[test]
fn issue3820_colon_is_never_a_valid_option_letter_noisy() {
    for (label, optstring) in [("colon-noisy", "a:"), ("colon-after-two", "a:b:")] {
        let script = format!("getopts '{optstring}' o -:; echo \"o=$o\"");
        let (stdout, stderr, code) = run_case(&script);
        assert_eq!(code, Some(0), "{label}: unexpected exit code");
        assert_eq!(stdout, "o=?\n", "{label}: stdout mismatch");
        assert!(
            stderr.contains("illegal option -- :"),
            "{label}: stderr lacks diagnostic: {stderr}"
        );
    }
}

#[test]
fn issue3820_colon_loop_and_ordinary_letter_control() {
    let (stdout, stderr, code) =
        run_case("while getopts '::a' o -: -a; do echo \"o=$o\"; done; echo done");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "o=?\no=a\ndone\n");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");

    let (stdout, stderr, code) = run_case("getopts '::a' o -a; echo \"o=$o\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "o=a\n");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

#[test]
fn issue3820_declared_qmark_sets_optarg_in_silent_mode() {
    let (stdout, stderr, code) = run_case("getopts ':?' o '-?'; echo \"o=$o OPTARG=[$OPTARG]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "o=? OPTARG=[?]\n");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");

    let (stdout, stderr, code) =
        run_case("OPTARG=old; getopts ':?' o '-?'; echo \"arg=[$OPTARG]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "arg=[?]\n");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");

    // Even with ':' after '?' in optstring, dogetopts sees ret == '?' and
    // sets OPTARG to '?' in silent mode when the argument is present.
    let (stdout, stderr, code) =
        run_case("getopts ':?:' o '-?' arg; echo \"o=$o OPTARG=[$OPTARG]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "o=? OPTARG=[?]\n");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

#[test]
fn issue3820_declared_qmark_unbinds_optarg_in_noisy_mode() {
    let (stdout, stderr, code) =
        run_case("OPTARG=old; getopts '?' o '-?'; echo \"o=$o\"; declare -p OPTARG 2>&1");
    assert_eq!(code, Some(1));
    assert!(stdout.starts_with("o=?\n"), "stdout: {stdout}");
    assert!(
        stdout.contains("OPTARG: not found"),
        "OPTARG should be unbound, stdout: {stdout}"
    );
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

#[test]
fn issue3820_undeclared_qmark_controls() {
    let (stdout, stderr, code) = run_case("getopts ':ab' o '-?'; echo \"o=$o OPTARG=[$OPTARG]\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "o=? OPTARG=[?]\n");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");

    let (stdout, stderr, code) = run_case("getopts 'ab' o '-?'; echo \"o=$o\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "o=?\n");
    assert!(
        stderr.contains("illegal option -- ?"),
        "stderr lacks diagnostic: {stderr}"
    );
}
