//! rubash#547: `bash -o` / `bash +o` with no option argument is the list
//! form, not a usage error.
//!
//! GNU shell.c routes the argument-less `-o'/`+o' to the same listing
//! `set -o` / `set +o` prints (options.c print_all_shell_options). WSL GNU
//! 5.3.0(1) probes, 2026-10-12:
//!   `bash -o`  -> the `name\tstate` table (allexport first), rc 0
//!   `bash +o`  -> `set +o name` / `set -o name` recreate lines, rc 0
//! The old rubash failed with `-o: option requires an argument`, rc 2.

use std::process::Command;

fn run(args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("__RUBASH_TRAP_ORIG_IGN")
        .args(args)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn dash_o_without_argument_lists_the_option_table() {
    let (stdout, stderr, rc) = run(&["-o"]);
    // GNU print_all_shell_options pads the name column; the exact padding
    // is locked by the byte-for-byte equality test below.
    assert!(stdout.starts_with("allexport"), "stdout head: {stdout:?}");
    let names: Vec<&str> = stdout
        .lines()
        .map(|line| line.split('\t').next().unwrap_or("").trim_end())
        .collect();
    assert!(names.contains(&"braceexpand"));
    assert!(names.contains(&"xtrace"));
    assert!(stdout
        .lines()
        .all(|line| line.ends_with("\toff") || line.ends_with("\ton")));
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
}

#[test]
fn plus_o_without_argument_lists_the_recreate_commands() {
    let (stdout, stderr, rc) = run(&["+o"]);
    assert!(
        stdout.starts_with("set +o allexport\n"),
        "stdout head: {stdout:?}"
    );
    assert!(stdout.contains("set -o braceexpand\n"));
    assert!(stdout.contains("set +o xtrace\n"), "tail: {stdout:?}");
    assert_eq!(stderr, "");
    assert_eq!(rc, Some(0));
}

#[test]
fn dash_o_matches_set_dash_o_byte_for_byte() {
    let (bash_o_stdout, _, _) = run(&["-o"]);
    let (set_o_stdout, _, _) = run(&["-c", "set -o"]);
    assert_eq!(bash_o_stdout, set_o_stdout);

    let (plus_o_stdout, _, _) = run(&["+o"]);
    let (set_plus_o_stdout, _, _) = run(&["-c", "set +o"]);
    assert_eq!(plus_o_stdout, set_plus_o_stdout);
}

#[test]
fn dash_o_with_a_bad_name_still_fails() {
    // The list form covers only the MISSING-argument case; a named
    // non-option keeps GNU's `bash: line 0: bash: badname: invalid option
    // name`, exit 2.
    let (stdout, stderr, rc) = run(&["-o", "badname"]);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "bash: line 0: bash: badname: invalid option name\n");
    assert_eq!(rc, Some(2));
}

#[test]
fn dash_o_with_a_real_option_still_sets_it() {
    let (stdout, stderr, rc) = run(&["-o", "nounset", "-c", "echo ${unset_var}"]);
    // The nounset expansion error aborts the echo before it prints; the
    // nounset fatal exit code itself is outside this ticket — what the
    // `-o NAME` form must do is APPLY the option (the unbound-variable
    // diagnostic fires at all).
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("unset_var: unbound variable"),
        "stderr: {stderr:?}"
    );
    assert_ne!(rc, Some(0));
}
