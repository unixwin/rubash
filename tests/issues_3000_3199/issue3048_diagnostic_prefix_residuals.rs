//! rubash#3048: residual private `diagnostic_prefix` copies still
//! pair the per-frame line with the TOP-LEVEL script name, so an
//! error inside a function defined in a sourced file is reported
//! under the top-level script's name with the definition file's
//! line. GNU reports the DEFINITION file.
//!
//! GNU ground truth (bash 5.3.20, verified by script-file probes;
//! every expectation below was captured from the oracle): builtin
//! diagnostics go through builtin_error -> error_prolog
//! (builtins/common.c:83-94), whose name is `get_name_for_error`
//! (error.c:90-120) = BASH_SOURCE[0]; a call frame's BASH_SOURCE
//! entry is the function's DEFINITION file (execute_cmd.c:5310-5313
//! execute_function pushes the recorded source_file).
//!
//! rubash#3035/#3047 landed the `__RUBASH_CURRENT_FILE` carrier
//! (stamped per command next to `__RUBASH_CURRENT_LINE`, with a
//! process-env mirror) and the shared helper
//! `src/builtins/diagnostic.rs`, and converted six copies. The
//! copies pinned here were left behind (file faces occupied by
//! in-flight lanes, or std::env signatures):
//!   printf.rs:270 (+ readonly variant :283), set/unset.rs:295,
//!   set.rs:127 Prolog, declare/diagnostic.rs:9,
//!   setattr/value.rs:183, kill.rs:1911, alias.rs:255,
//!   help.rs:516, shopt.rs:84 Prolog, shift.rs:50 / exit.rs:31
//!   Prolog constructions.
//! Each still reads the top-level-only `__RUBASH_SCRIPT_NAME`.
//!
//! Fixture shape (per rubash#3035's pin): each lib function is
//! defined on line 1 of ./lib.sh, so every GNU expectation reads
//! `./lib.sh: line 1: ...`; the base instead prints the driver's
//! name with that same line 1.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, files: &[(&str, &str)], driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3048-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    for (name, body) in files {
        let mut file = std::fs::File::create(dir.join(name)).expect("write fixture");
        file.write_all(body.as_bytes()).expect("write fixture");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(driver)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// One sourced-lib function + a driver that sources and calls it,
/// echoing the builtin's status. Returns (stdout, stderr, rc).
fn run_lib_fn(tag: &str, lib_fn: &str, call: &str, prelude: &str) -> (String, String, Option<i32>) {
    let driver = format!("{prelude}source ./lib.sh\n{call}\necho \"rc=$?\"\n");
    run_driver(
        tag,
        &[("lib.sh", lib_fn), ("drv.sh", driver.as_str())],
        "drv.sh",
    )
}

/// printf's numeric-conversion error (printf.rs:270 copy).
#[test]
fn printf_invalid_number_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("printf", "pf() { printf '%d\\n' abc; }\n", "pf", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "0\nrc=1\n");
    assert_eq!(stderr, "./lib.sh: line 1: printf: abc: invalid number\n");
}

/// printf -v onto a readonly variable (printf.rs:283 readonly
/// variant of the copy). GNU's report_error family prolog still
/// names the definition file.
#[test]
fn printf_v_readonly_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn(
        "printf-ro",
        "pfv() { printf -v rovar 'x'; }\n",
        "pfv",
        "rovar=init\nreadonly rovar\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "./lib.sh: line 1: rovar: readonly variable\n");
}

/// unset's invalid-option error + usage (set/unset.rs:295 copy).
#[test]
fn unset_invalid_option_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("unset", "us() { unset -Z; }\n", "us", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=2\n");
    assert_eq!(
        stderr,
        "./lib.sh: line 1: unset: -Z: invalid option\nunset: usage: unset [-f] [-v] [-n] [name ...]\n"
    );
}

/// set -o with an unknown option name (set.rs:127 Prolog copy).
#[test]
fn set_invalid_option_name_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("set", "st() { set -o nosuchopt3048; }\n", "st", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=2\n");
    assert_eq!(
        stderr,
        "./lib.sh: line 1: set: nosuchopt3048: invalid option name\n"
    );
}

/// declare with an invalid identifier (declare/diagnostic.rs:9).
#[test]
fn declare_invalid_identifier_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("declare", "dc() { declare 9bad=v; }\n", "dc", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "./lib.sh: line 1: declare: `9bad=v': not a valid identifier\n"
    );
}

/// readonly with an invalid identifier, via the setattr apply
/// path (setattr/value.rs:183 copy).
#[test]
fn readonly_invalid_identifier_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("readonly", "ro() { readonly 9bad=v; }\n", "ro", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "./lib.sh: line 1: readonly: `9bad=v': not a valid identifier\n"
    );
}

/// kill with an unknown signal spec (kill.rs:1911 copy).
#[test]
fn kill_invalid_signal_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("kill", "kl() { kill -Z; }\n", "kl", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "./lib.sh: line 1: kill: Z: invalid signal specification\n"
    );
}

/// kill with a non-pid operand (kill.rs:1911 copy, BadPid arm).
#[test]
fn kill_bad_pid_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("killpid", "kl2() { kill abc; }\n", "kl2", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "./lib.sh: line 1: kill: `abc': not a pid or valid job spec\n"
    );
}

/// alias lookup miss (alias.rs:255 copy).
#[test]
fn alias_not_found_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("alias", "al() { alias nosuch3048; }\n", "al", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "./lib.sh: line 1: alias: nosuch3048: not found\n");
}

/// help with no matching topic (help.rs:516 copy).
#[test]
fn help_no_topics_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("help", "hp() { help nosuch3048; }\n", "hp", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "./lib.sh: line 1: help: no help topics match `nosuch3048'.  Try `help help' or `man -k nosuch3048' or `info nosuch3048'.\n"
    );
}

/// shift with a non-numeric count (shift.rs Prolog construction).
/// (An over-range shift COUNT is silent in GNU — rc 1, no
/// diagnostic — so it cannot exercise the prefix; the numeric
/// arms below are the prefix-bearing shapes.)
#[test]
fn shift_numeric_arg_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("shift-nan", "sfa() { shift abc; }\n", "sfa", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=2\n");
    assert_eq!(
        stderr,
        "./lib.sh: line 1: shift: abc: numeric argument required\n"
    );
}

/// shift with a negative count (shift.rs Prolog, Erange arm).
#[test]
fn shift_negative_count_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("shift-neg", "sfn() { shift -1; }\n", "sfn", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "./lib.sh: line 1: shift: -1: shift count out of range\n"
    );
}

/// exit with a non-numeric status (exit.rs Prolog construction).
/// GNU does NOT terminate the shell on this error (verified in
/// functions and at top level): the builtin returns 2 and the
/// script continues — the driver asserts the continuation line.
#[test]
fn exit_numeric_arg_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_driver(
        "exit",
        &[
            ("lib.sh", "ex() { exit abc; }\n"),
            ("drv.sh", "source ./lib.sh\nex\necho \"after rc=$?\"\n"),
        ],
        "drv.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "after rc=2\n");
    assert_eq!(
        stderr,
        "./lib.sh: line 1: exit: abc: numeric argument required\n"
    );
}

/// shopt -s with an unknown shell option name (shopt.rs:84
/// Prolog copy).
#[test]
fn shopt_invalid_option_name_in_sourced_function_reports_definition_file() {
    let (stdout, stderr, rc) = run_lib_fn("shopt", "so() { shopt -s nosuchopt3048; }\n", "so", "");
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "./lib.sh: line 1: shopt: nosuchopt3048: invalid shell option name\n"
    );
}

/// Control: a function defined in the top-level script itself
/// reports the script's own name (no over-correction).
#[test]
fn top_level_function_alias_error_reports_own_file() {
    let (stdout, stderr, rc) = run_driver(
        "self",
        &[(
            "self.sh",
            "al2() { alias nosuch3048; }\nal2\necho \"rc=$?\"\n",
        )],
        "self.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "self.sh: line 1: alias: nosuch3048: not found\n");
}

/// Control: a direct top-level error (no function frame) reports
/// the script's own name and line.
#[test]
fn top_level_alias_error_reports_own_file() {
    let (stdout, stderr, rc) = run_driver("top", &[("top.sh", "alias nosuch3048\n")], "top.sh");
    assert_eq!(rc, Some(1));
    assert_eq!(stdout, "");
    assert_eq!(stderr, "top.sh: line 1: alias: nosuch3048: not found\n");
}
