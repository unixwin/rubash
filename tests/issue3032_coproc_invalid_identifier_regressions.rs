//! rubash#3032 — coproc NAME is expanded before the identifier check,
//! and an unset `$namevar` reports `(null)`, not the unexpanded name.
//!
//! GNU execute_cmd.c:2508-2516 execute_coproc expands the NAME word
//! unsplit (expand_string_unsplit_to_string) before valid_identifier:
//! an unset `$namevar` expands to NO word, so the expansion returns NULL
//! (subst.c:3848-3865) and err_invalidid prints `(null)` (error.c:461).
//! A quoted empty word (`""` / `"$namevar"`) keeps one empty word
//! (subst.c expand_word_internal, finished_with_string), so it reports
//! `''`; a double-quoted `"$@"` with no positional parameters is the
//! single exception that still discards to NULL (quoted_dollar_at).
//! The SET case binds the expanded name, like GNU.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings, audit-tree
//! oracle binary); the fd numbers inside a coproc array are
//! kernel-assigned, so the set-case pin asserts the array's shape, not
//! its values.

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
fn coproc_unset_namevar_reports_null() {
    let (stdout, stderr, code) = rubash("coproc $namevar { foo; }; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("`(null)': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_braced_unset_namevar_reports_null() {
    let (stdout, stderr, code) = rubash("coproc ${namevar} { foo; }; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("`(null)': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_empty_default_unset_namevar_reports_null() {
    let (stdout, stderr, code) = rubash("coproc ${namevar:-} { foo; }; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("`(null)': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_empty_command_substitution_name_reports_null() {
    let (stdout, stderr, code) = rubash("coproc $(:) { foo; }; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("`(null)': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_double_quoted_at_with_no_params_reports_null() {
    let (stdout, stderr, code) = rubash("coproc \"$@\" { foo; }; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("`(null)': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_empty_quoted_name_reports_empty() {
    let (stdout, stderr, code) = rubash("coproc \"\" { foo; }; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("`'': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_double_quoted_unset_namevar_reports_empty() {
    let (stdout, stderr, code) = rubash("coproc \"$namevar\" { foo; }; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("`'': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_double_quoted_star_with_no_params_reports_empty() {
    let (stdout, stderr, code) = rubash("coproc \"$*\" { foo; }; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("`'': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_set_namevar_binds_expanded_name() {
    let (stdout, stderr, code) =
        rubash("namevar=FOO; coproc $namevar { sleep 0.2; }; declare -p FOO; sleep 0.4");
    assert!(
        stdout.starts_with("declare -a FOO=([0]=\"") && stdout.contains("[1]=\""),
        "stdout: {stdout}"
    );
    assert!(
        !stderr.contains("not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn coproc_at_with_params_reports_joined_name() {
    let (stdout, stderr, code) = rubash("set -- a b; coproc $@ { foo; }; echo rc=$?");
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("`a b': not a valid identifier"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
