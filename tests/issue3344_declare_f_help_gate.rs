//! rubash#3344 — `command declare -f --help` prints no help and
//! returns 1, where GNU prints the long help with status 2.
//!
//! GNU law: declare_internal's getopt loop carries CASE_HELPOPT
//! (builtins/declare.def:362), so `--help` in the option region
//! fires before any function/variable branch runs — `-f` only sets
//! att_function in the flags, it never diverts before the help
//! decision (builtins/common.h:29-41 CHECK_HELPOPT). Rubash's plain
//! path reaches the shared gate first (builtin_help_gate,
//! src/executor/command_dispatch.rs:829, rubash#2753), so plain
//! `declare -f --help` is already correct; but the `command`
//! re-dispatch path (execute_command_without_aliases) carries no
//! gate, and execute_declare_command's `-f` diversion
//! (declare_has_function_option) runs before the core builtin's
//! own BuiltinOpt help scan, so the function face treats `--help`
//! as a function name to look up (rc 1, no help).
//!
//! Expectations verified against GNU bash 5.3.20 (built from
//! third_party/bash @9c465866); the help texts below are
//! byte-identical between GNU and rubash.

use std::process::Command;

const DECLARE_HELP: &str = "declare: declare [-aAfFgiIlnrtux] [name[=value] ...] or declare -p [-aAfFilnrtux] [name ...]\n    Set variable values and attributes.\n    \n    Declare variables and give them attributes.  If no NAMEs are given,\n    display the attributes and values of all variables.\n    \n    Options:\n      -f\trestrict action or display to function names and definitions\n      -F\trestrict display to function names only (plus line number and\n    \t\tsource file when debugging)\n      -g\tcreate global variables when used in a shell function; otherwise\n    \t\tignored\n      -I\tif creating a local variable, inherit the attributes and value\n    \t\tof a variable with the same name at a previous scope\n      -p\tdisplay the attributes and value of each NAME\n    \n    Options which set attributes:\n      -a\tto make NAMEs indexed arrays (if supported)\n      -A\tto make NAMEs associative arrays (if supported)\n      -i\tto make NAMEs have the `integer' attribute\n      -l\tto convert the value of each NAME to lower case on assignment\n      -n\tmake NAME a reference to the variable named by its value\n      -r\tto make NAMEs readonly\n      -t\tto make NAMEs have the `trace' attribute\n      -u\tto convert the value of each NAME to upper case on assignment\n      -x\tto make NAMEs export\n    \n    Using `+' instead of `-' turns off the given attribute, except for a,\n    A, and r.\n    \n    Variables with the integer attribute have arithmetic evaluation (see\n    the `let' command) performed when the variable is assigned a value.\n    \n    When used in a function, `declare' makes NAMEs local, as with the `local'\n    command.  The `-g' option suppresses this behavior.\n    \n    Exit Status:\n    Returns success unless an invalid option is supplied or a variable\n    assignment error occurs.\n";
const TYPESET_HELP: &str = "typeset: typeset [-aAfFgiIlnrtux] name[=value] ... or typeset -p [-aAfFilnrtux] [name ...]\n    Set variable values and attributes.\n    \n    A synonym for `declare'.  See `help declare'.\n";

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
fn issue3344_command_declare_f_help() {
    let (stdout, stderr, code) = rubash("command declare -f --help");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, DECLARE_HELP);
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_command_declare_cap_f_help() {
    let (stdout, stderr, code) = rubash("command declare -F --help");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, DECLARE_HELP);
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_command_typeset_f_help() {
    let (stdout, stderr, code) = rubash("command typeset -f --help");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, TYPESET_HELP);
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_command_declare_fx_help() {
    // A second attribute letter before --help: GNU's getopt sets
    // att_exported, then CASE_HELPOPT still fires.
    let (stdout, stderr, code) = rubash("command declare -fx --help");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, DECLARE_HELP);
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_command_declare_f_help_in_function() {
    let (stdout, stderr, code) = rubash("g() { command declare -f --help; }; g");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, DECLARE_HELP);
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_plain_declare_f_help() {
    // Control: the plain path already reaches the shared gate.
    let (stdout, stderr, code) = rubash("declare -f --help");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, DECLARE_HELP);
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_command_declare_help() {
    // Control: no -f diversion on this shape.
    let (stdout, stderr, code) = rubash("command declare --help");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, DECLARE_HELP);
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_builtin_declare_f_help() {
    // Control: the builtin re-dispatch applies the shared gate.
    let (stdout, stderr, code) = rubash("builtin declare -f --help");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, DECLARE_HELP);
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_command_declare_f_operand_help_is_name() {
    // --help AFTER an operand is a function name, not help
    // (bashgetopt.c:77-84 stops the scan at the first non-option
    // word); GNU: rc 1, no help text, no diagnostic.
    let (stdout, stderr, code) = rubash("command declare -f foo --help; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_command_declare_f_dashdash_help_is_name() {
    // --help after -- is an operand too.
    let (stdout, stderr, code) = rubash("command declare -f -- --help; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_command_declare_f_invalid_before_help() {
    // An invalid letter before --help ends the scan in error
    // (GNU: -Z invalid option + usage, EX_USAGE).
    let (stdout, stderr, code) = rubash("command declare -fZ --help");
    assert_eq!(code, Some(2));
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("declare: -Z: invalid option"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("declare: usage: declare [-aAfFgiIlnrtux]"),
        "stderr was: {stderr}"
    );
}

#[test]
fn issue3344_command_declare_f_name_then_help() {
    // The found function prints, the --help name fails: rc 1,
    // no help text.
    let (stdout, stderr, code) = rubash("f() { :; }; command declare -f f --help; echo rc=$?");
    assert_eq!(code, Some(0));
    assert!(stdout.contains("f ()"), "stdout was: {stdout}");
    assert!(stdout.contains("rc=1\n"), "stdout was: {stdout}");
    assert!(
        !stdout.contains("Set variable values and attributes"),
        "help text leaked into stdout: {stdout}"
    );
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_command_declare_f_still_lists() {
    // The -f diversion itself still works when no help is asked.
    let (stdout, stderr, code) = rubash("f() { :; }; command declare -f f; echo rc=$?");
    assert_eq!(code, Some(0));
    assert!(stdout.contains("f ()"), "stdout was: {stdout}");
    assert!(stdout.contains("rc=0\n"), "stdout was: {stdout}");
    assert_eq!(stderr, "");
}

#[test]
fn issue3344_command_declare_f_no_functions() {
    let (stdout, stderr, code) = rubash("command declare -f; echo rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
}
