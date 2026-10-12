//! rubash#544: `declare -aA` (indexed + associative set on one command
//! line) is a usage error.
//!
//! GNU builtins/declare.def:222 declare_invalid_opts (bash 5.3): `-a'
//! together with `-A' — combined (`-aA') or separate (`-a -A') — is
//! rejected naming `-a' as the invalid option, with EX_BADUSAGE and no
//! usage text; the same check rejects an attribute set AND unset on one
//! command line (`-A +A' names `-A', `-a +a' names `-a'). `+aA' is NOT a
//! conflict (`+a' unsets — nothing is set twice) and stays rc 0. The
//! display path returns before the check (declare.def:330-406 -> 408), so
//! `-paA' stays a pure display, and with no name operands declare returns
//! before it too (`declare -aA` alone is rc 0). No variable is created on
//! the error path (`declare -aA x; declare -p x` -> "x: not found").
//!
//! WSL GNU 5.3.0(1) probes, 2026-10-12:
//!   `declare -aA x`    -> `declare: -a: invalid option`, rc 2
//!   `declare -paA x`   -> `declare: x: not found`, rc 1
//!   `declare -aA`      -> rc 0, silent
//!   `declare -A +A x`  -> `declare: -A: invalid option`, rc 2

use std::process::Command;

fn run(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("__RUBASH_TRAP_ORIG_IGN")
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
fn combined_aA_with_name_names_minus_a() {
    let (stdout, stderr, rc) = run("declare -aA x");
    assert_eq!(stdout, "");
    assert!(
        stderr.ends_with("declare: -a: invalid option\n"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(2));
}

#[test]
fn separate_a_dash_A_with_name_names_minus_a() {
    let (stdout, stderr, rc) = run("declare -a -A x");
    assert_eq!(stdout, "");
    assert!(
        stderr.ends_with("declare: -a: invalid option\n"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(2));
}

#[test]
fn no_variable_is_created_on_the_error_path() {
    let (stdout, stderr, rc) = run("declare -aA x; declare -p x");
    assert_eq!(stdout, "");
    assert!(
        stderr.ends_with("declare: x: not found\n"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(1));
}

#[test]
fn display_path_stays_a_pure_display() {
    // `-paA` returns before the conflict check in GNU: `-p' displays, the
    // unset variable reports `x: not found`, rc 1.
    let (stdout, stderr, rc) = run("declare -paA x");
    assert_eq!(stdout, "");
    assert!(
        stderr.ends_with("declare: x: not found\n"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(1));
}

#[test]
fn no_name_operands_is_a_silent_success() {
    let (stdout, stderr, rc) = run("declare -aA");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
}

#[test]
fn set_and_unset_of_the_same_attribute_names_that_attribute() {
    let (stdout, stderr, rc) = run("declare -A +A x");
    assert_eq!(stdout, "");
    assert!(
        stderr.ends_with("declare: -A: invalid option\n"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(2));

    let (stdout, stderr, rc) = run("declare -a +a x");
    assert_eq!(stdout, "");
    assert!(
        stderr.ends_with("declare: -a: invalid option\n"),
        "stderr: {stderr:?}"
    );
    assert_eq!(rc, Some(2));
}

#[test]
fn plus_a_unset_is_not_a_conflict() {
    // `+aA' UNSETS the indexed attribute while setting the associative one
    // — declare.def's check rejects only same-sign combinations (`-aA')
    // and set+unset of the SAME attribute (`-a +a'). GNU 5.3.0 probe:
    // `declare +aA x` is silent, rc 0.
    let (stdout, stderr, rc) = run("declare +aA x");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
}
