//! rubash#3345 — `declare -p` missing-name diagnostics must be reported
//! exactly once each, in operand order.
//!
//! GNU builtins/declare.def:389-406: with -p the builtin walks the operand
//! list in order and each missing name is reported once, right where it
//! occurs (show_name_attributes + sh_notfound). An option-shaped operand
//! (a valid option letter after the first operand, `--`, `--help`, `+x`)
//! is a NAME to look up, never an option; an invalid-identifier operand
//! (`X(BR)`, `a[]`, `a[bad`, `X(BR)=1`) is likewise a name, reported
//! `not found`.
//!
//! The pre-fix tree reported every invalid-identifier operand twice: once
//! from the core's validation loop (before every other diagnostic) and once
//! from the print loop (in operand order). `declare -p x --help` reported
//! `--help` twice; `declare -p nope1 -x nope2` reported `-x` first and
//! then again in place.
//!
//! Expectations verified against GNU bash 5.3.20 with script files.
//! stdout and stderr are asserted separately: the merged-stream
//! interleave (a print between two diagnostics on one `2>&1` stream) is
//! the repo-wide buffered-output limitation, not this lane.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3345-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("t.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("t.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn declare_p_option_shaped_names_report_once_in_operand_order() {
    let script = "declare -p nope1 -x nope2; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("order", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: nope1: not found\n\
         t.sh: line 1: declare: -x: not found\n\
         t.sh: line 1: declare: nope2: not found\n"
    );
}

#[test]
fn declare_p_two_plain_names_then_option_shaped() {
    let script = "declare -p nope1 nope2 -x; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("plain-then-opt", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: nope1: not found\n\
         t.sh: line 1: declare: nope2: not found\n\
         t.sh: line 1: declare: -x: not found\n"
    );
}

#[test]
fn declare_p_mixed_found_and_missing_names() {
    let script = "v=1\nw=hello\ndeclare -p v nope1 -x nope2 w; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("mixed", script);
    assert_eq!(stdout, "declare -- v=\"1\"\ndeclare -- w=\"hello\"\nrc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 3: declare: nope1: not found\n\
         t.sh: line 3: declare: -x: not found\n\
         t.sh: line 3: declare: nope2: not found\n"
    );
}

#[test]
fn declare_p_help_literal_reported_once_after_operand() {
    let script = "declare -p x --help; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("help", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: x: not found\n\
         t.sh: line 1: declare: --help: not found\n"
    );
}

#[test]
fn declare_p_help_literal_between_plain_names() {
    let script = "declare -p nope1 --help nope2; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("help-mid", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: nope1: not found\n\
         t.sh: line 1: declare: --help: not found\n\
         t.sh: line 1: declare: nope2: not found\n"
    );
}

#[test]
fn declare_p_repeated_help_literals_each_reported_once() {
    let script = "declare -p nope1 --help --help; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("help-twice", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: nope1: not found\n\
         t.sh: line 1: declare: --help: not found\n\
         t.sh: line 1: declare: --help: not found\n"
    );
}

#[test]
fn declare_p_double_dash_operand_in_order() {
    let script = "declare -p nope1 -- nope2; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("dashdash", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: nope1: not found\n\
         t.sh: line 1: declare: --: not found\n\
         t.sh: line 1: declare: nope2: not found\n"
    );
}

#[test]
fn declare_p_double_dash_alone_is_one_operand() {
    let script = "declare -p -- --; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("dashdash-alone", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(stderr, "t.sh: line 1: declare: --: not found\n");
}

#[test]
fn declare_p_option_letters_after_operand_are_names() {
    let script =
        "declare -p -p nope1 -a nope2; echo rc:$?\ndeclare -p nope1 -p nope2; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("opt-letters", script);
    assert_eq!(stdout, "rc:1\nrc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: nope1: not found\n\
         t.sh: line 1: declare: -a: not found\n\
         t.sh: line 1: declare: nope2: not found\n\
         t.sh: line 2: declare: nope1: not found\n\
         t.sh: line 2: declare: -p: not found\n\
         t.sh: line 2: declare: nope2: not found\n"
    );
}

#[test]
fn declare_p_found_name_then_option_shaped() {
    let script = "v=1\ndeclare -p v -x; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("found-then-opt", script);
    assert_eq!(stdout, "declare -- v=\"1\"\nrc:1\n");
    assert_eq!(stderr, "t.sh: line 2: declare: -x: not found\n");
}

#[test]
fn declare_p_option_end_marker_then_option_shaped() {
    let script = "declare -p -- -x nope1; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("marker", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: -x: not found\n\
         t.sh: line 1: declare: nope1: not found\n"
    );
}

#[test]
fn declare_p_plus_form_option_shaped_name() {
    let script = "declare -p nope1 +x; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("plus", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: nope1: not found\n\
         t.sh: line 1: declare: +x: not found\n"
    );
}

#[test]
fn declare_p_single_invalid_operand_reported_once() {
    let script = "declare -p 'X(BR)'; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("single-invalid", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(stderr, "t.sh: line 1: declare: X(BR): not found\n");
}

#[test]
fn declare_p_all_invalid_operands_still_reported_in_order() {
    let script = "declare -p 'X(BR)' 'Y(BR)'; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("all-invalid", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: X(BR): not found\n\
         t.sh: line 1: declare: Y(BR): not found\n"
    );
}

#[test]
fn declare_p_valued_operand_reported_once() {
    let script = "declare -p dv=6; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("valued", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(stderr, "t.sh: line 1: declare: dv=6: not found\n");
}

#[test]
fn declare_p_subscript_names_in_operand_order() {
    let script = "declare -p nope1 'a[1]'; echo rc:$?\ndeclare -p 'a[1]' nope1; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("subscript", script);
    assert_eq!(stdout, "rc:1\nrc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: nope1: not found\n\
         t.sh: line 1: declare: a[1]: not found\n\
         t.sh: line 2: declare: a[1]: not found\n\
         t.sh: line 2: declare: nope1: not found\n"
    );
}

#[test]
fn declare_p_bad_subscript_names_reported_once() {
    let script = "declare -p 'a[]' nope1; echo rc:$?\ndeclare -p 'a[bad'; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("bad-subscript", script);
    assert_eq!(stdout, "rc:1\nrc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: a[]: not found\n\
         t.sh: line 1: declare: nope1: not found\n\
         t.sh: line 2: declare: a[bad: not found\n"
    );
}

#[test]
fn declare_p_invalid_valued_operand_reported_once() {
    let script = "declare -p 'X(BR)=1' nope1; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("invalid-valued", script);
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: X(BR)=1: not found\n\
         t.sh: line 1: declare: nope1: not found\n"
    );
}

#[test]
fn declare_p_attribute_flags_after_operand_are_names() {
    let script = "declare -p -a nope1 -A nope2; echo rc:$?\ndeclare -p nope1 -a; echo rc:$?\ndeclare -p nope1 -n nope2; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("attr-flags", script);
    assert_eq!(stdout, "rc:1\nrc:1\nrc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: nope1: not found\n\
         t.sh: line 1: declare: -A: not found\n\
         t.sh: line 1: declare: nope2: not found\n\
         t.sh: line 2: declare: nope1: not found\n\
         t.sh: line 2: declare: -a: not found\n\
         t.sh: line 3: declare: nope1: not found\n\
         t.sh: line 3: declare: -n: not found\n\
         t.sh: line 3: declare: nope2: not found\n"
    );
}

#[test]
fn typeset_p_shares_the_report_order() {
    let script = "typeset -p nope1 -x nope2; echo rc:$?\ntypeset -p x --help; echo rc:$?\n";
    let (stdout, stderr, _) = rubash_file("typeset", script);
    assert_eq!(stdout, "rc:1\nrc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: typeset: nope1: not found\n\
         t.sh: line 1: typeset: -x: not found\n\
         t.sh: line 1: typeset: nope2: not found\n\
         t.sh: line 2: typeset: x: not found\n\
         t.sh: line 2: typeset: --help: not found\n"
    );
}

#[test]
fn declare_p_missing_name_exits_failure() {
    let script = "declare -p nope1 -x nope2\n";
    let (stdout, stderr, code) = rubash_file("exit-code", script);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: nope1: not found\n\
         t.sh: line 1: declare: -x: not found\n\
         t.sh: line 1: declare: nope2: not found\n"
    );
    assert_eq!(code, Some(1));
}
