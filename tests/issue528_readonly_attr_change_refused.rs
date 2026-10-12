//! issue#528 regression: an attribute change on a readonly variable must be
//! refused, not silently applied. GNU bash protects more than the value:
//! declare.def:893 (with variables.h:126-128 valid/invalid_readonly_attrs)
//! refuses any request to set or clear the integer/array/assoc/case-mapping
//! attributes on a readonly variable — one `<cmd>: <name>: readonly variable`
//! diagnostic, declare exits 1, and the variable keeps its previous
//! attributes and value. Only export, local, nameref and trace bits stay
//! modifiable, an assignment on the same word leaves nothing applied
//! (declare.def:881), and the refusal is all-or-nothing for mixed requests
//! like `-ix`.
//!
//! Oracle: GNU bash 5.3.0 (WSL) 2026-10-12 — `readonly R=1; declare -i R;
//! declare -p R` errors `declare: R: readonly variable` (rc 1) and still
//! prints `declare -r R="1"`; rubash accepted the change silently and
//! printed `declare -ir R="1"`.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn shell_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

struct Output {
    status: i32,
    stdout: String,
    stderr: String,
}

fn run_c(script: &str) -> Output {
    let output = Command::new(shell_bin())
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    Output {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).replace('\r', ""),
    }
}

fn readonly_error_lines(stderr: &str) -> usize {
    stderr
        .lines()
        .filter(|l| l.contains("readonly variable"))
        .count()
}

/// The issue's exact repro: attribute silently becoming -ir is the bug.
#[test]
fn declare_i_on_readonly_is_refused() {
    let out = run_c("readonly R=1; declare -i R; echo rc=$?; declare -p R");
    assert_eq!(
        out.stdout, "rc=1\ndeclare -r R=\"1\"\n",
        "attr must not change"
    );
    assert_eq!(readonly_error_lines(&out.stderr), 1, "{:?}", out.stderr);
}

/// The array shape is refused too, and the scalar value is not converted.
#[test]
fn declare_a_on_readonly_is_refused_value_stays_scalar() {
    let out = run_c("readonly R11=11; declare -a R11; echo rc=$?; declare -p R11");
    assert_eq!(out.stdout, "rc=1\ndeclare -r R11=\"11\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 1, "{:?}", out.stderr);
}

/// Uppercase mapping refused on readonly.
#[test]
fn declare_u_on_readonly_is_refused() {
    let out = run_c("readonly R4=4; declare -u R4; echo rc=$?; declare -p R4");
    assert_eq!(out.stdout, "rc=1\ndeclare -r R4=\"4\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 1, "{:?}", out.stderr);
}

/// Re-requesting -i on an already -ir readonly variable is still refused.
#[test]
fn declare_i_on_readonly_integer_is_refused() {
    let out = run_c("declare -ir X6=1; declare -i X6; echo rc=$?; declare -p X6");
    assert_eq!(out.stdout, "rc=1\ndeclare -ir X6=\"1\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 1, "{:?}", out.stderr);
}

/// Mixed -ix on readonly is all-or-nothing: neither attribute lands.
#[test]
fn mixed_ix_on_readonly_applies_nothing() {
    let out = run_c("readonly R7=7; declare -ix R7; echo rc=$?; declare -p R7");
    assert_eq!(out.stdout, "rc=1\ndeclare -r R7=\"7\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 1, "{:?}", out.stderr);
}

/// An assignment with a refused attribute leaves value AND attribute alone,
/// with exactly one diagnostic (GNU declare.def:881 skips the operand).
#[test]
fn assignment_with_refused_attr_changes_nothing() {
    let out = run_c("readonly R2=2; declare -i R2=9; echo rc=$?; declare -p R2");
    assert_eq!(out.stdout, "rc=1\ndeclare -r R2=\"2\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 1, "{:?}", out.stderr);
}

/// Clearing an attribute on a readonly variable is refused as well
/// (+i on a plain readonly has nothing to clear but is still invalid;
/// the +r removal keeps its existing refusal).
#[test]
fn declare_plus_i_on_readonly_is_refused() {
    let out = run_c("readonly R5=5; declare +i R5; echo rc=$?; declare -p R5");
    assert_eq!(out.stdout, "rc=1\ndeclare -r R5=\"5\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 1, "{:?}", out.stderr);
}

/// Export stays modifiable on a readonly variable (GNU allows -x; rc 0).
#[test]
fn declare_x_on_readonly_still_allowed() {
    let out = run_c("readonly R6=6; declare -x R6; echo rc=$?; declare -p R6");
    assert_eq!(out.stdout, "rc=0\ndeclare -rx R6=\"6\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 0, "{:?}", out.stderr);
}

/// Trace stays modifiable on a readonly variable (GNU allows -t; rc 0).
#[test]
fn declare_t_on_readonly_still_allowed() {
    let out = run_c("readonly R3=3; declare -t R3; echo rc=$?; declare -p R3");
    assert_eq!(out.stdout, "rc=0\ndeclare -rt R3=\"3\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 0, "{:?}", out.stderr);
}

/// Re-setting readonly on a readonly variable is a no-op success, and -p
/// display never triggers the gate.
#[test]
fn declare_r_and_p_on_readonly_still_allowed() {
    let out = run_c("readonly R=1; declare -r R; echo rc=$?; declare -p R");
    assert_eq!(out.stdout, "rc=0\ndeclare -r R=\"1\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 0, "{:?}", out.stderr);
}

/// The export builtin keeps working on readonly variables (regression guard).
#[test]
fn export_builtin_on_readonly_still_allowed() {
    let out = run_c("readonly R10=10; export R10; echo rc=$?; declare -p R10");
    assert_eq!(out.stdout, "rc=0\ndeclare -rx R10=\"10\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 0, "{:?}", out.stderr);
}

/// The +r removal keeps its existing refusal (regression guard).
#[test]
fn declare_plus_r_on_readonly_still_refused() {
    let out = run_c("readonly R5=5; declare +r R5; echo rc=$?; declare -p R5");
    assert_eq!(out.stdout, "rc=1\ndeclare -r R5=\"5\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 1, "{:?}", out.stderr);
}

/// A function-scope attribute declare on a global readonly errors exactly
/// once and leaves the global untouched.
#[test]
fn function_scope_declare_i_on_readonly_errors_once() {
    let out = run_c("readonly R8=8; f(){ declare -i R8; }; f; echo rc=$?; declare -p R8");
    assert_eq!(out.stdout, "rc=1\ndeclare -r R8=\"8\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 1, "{:?}", out.stderr);
}

/// local keeps its existing refusal (regression guard, one diagnostic).
#[test]
fn local_i_on_readonly_still_refused_once() {
    let out = run_c("readonly R9=9; f(){ local -i R9; }; f; declare -p R9");
    assert_eq!(readonly_error_lines(&out.stderr), 1, "{:?}", out.stderr);
    assert_eq!(out.stdout, "declare -r R9=\"9\"\n");
}

/// Plain assignment through declare keeps its existing refusal (one
/// diagnostic, no attribute requested).
#[test]
fn declare_assignment_on_readonly_still_refused_once() {
    let out = run_c("readonly R2=2; declare R2=9; echo rc=$?; declare -p R2");
    assert_eq!(out.stdout, "rc=1\ndeclare -r R2=\"2\"\n");
    assert_eq!(readonly_error_lines(&out.stderr), 1, "{:?}", out.stderr);
}
