//! rubash#2219 — a literal quoted empty operand inside a command
//! substitution (`$(cd "" && pwd)`) silently performed the operand-less
//! $HOME move, where GNU bash refuses with "null directory"
//! (builtins/cd.def:333: an operand WORD that is empty is a refusal;
//! the $HOME move is only for NO word at all, cd.def:316).
//!
//! Root cause: the `$(cd X && pwd)` fast path reads its operand from
//! split_shell_words, which dropped quoted empty words entirely, so
//! `cd ""` reached the fast path as an operand-less cd. The split now
//! reports the empty word, and the fast path declines every
//! expands-to-nothing operand to the real cd/pwd pair, whose verdicts
//! are GNU's: quoted empty → null directory; unquoted empty (the word
//! is removed by expansion) → the $HOME move.
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! probes in the cluster workdir: probe-gnu-2219.sh H/K/N rows).

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

fn home() -> String {
    std::env::var("HOME").expect("HOME set in test env")
}

#[test]
fn issue2219_comsub_double_quoted_empty_operand_is_null_directory() {
    let (stdout, stderr, code) =
        rubash("cd /tmp; x=$(cd \"\" && pwd); echo \"x=[$x] rc=$?\"; echo \"PWD=$PWD\"");
    assert_eq!(stdout, "x=[] rc=1\nPWD=/tmp\n");
    assert!(stderr.contains("cd: null directory"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2219_comsub_single_quoted_empty_operand_is_null_directory() {
    let (stdout, stderr, _code) =
        rubash("cd /tmp; x=$(cd '' && pwd); echo \"x=[$x] rc=$?\"; echo \"PWD=$PWD\"");
    assert_eq!(stdout, "x=[] rc=1\nPWD=/tmp\n");
    assert!(stderr.contains("cd: null directory"), "stderr: {stderr}");
}

#[test]
fn issue2219_comsub_empty_operand_with_pwd_physical_is_null_directory() {
    let (stdout, stderr, _code) = rubash("cd /tmp; x=$(cd '' && pwd -P); echo \"x=[$x] rc=$?\"");
    assert_eq!(stdout, "x=[] rc=1\n");
    assert!(stderr.contains("cd: null directory"), "stderr: {stderr}");
}

#[test]
fn issue2219_comsub_quoted_empty_variable_is_null_directory() {
    let (stdout, stderr, _code) =
        rubash("cd /tmp; e=; x=$(cd \"$e\" && pwd); echo \"x=[$x] rc=$?\"");
    assert_eq!(stdout, "x=[] rc=1\n");
    assert!(stderr.contains("cd: null directory"), "stderr: {stderr}");
}

#[test]
fn issue2219_comsub_unquoted_empty_variable_is_the_home_move() {
    // Unquoted, the empty expansion removes the word: no operand at
    // all remains, so GNU performs the operand-less $HOME move.
    let (stdout, stderr, _code) = rubash("cd /tmp; e=; x=$(cd $e && pwd); echo \"x=[$x] rc=$?\"");
    assert_eq!(stdout, format!("x=[{}] rc=0\n", home()));
    assert_eq!(stderr, "");
}

#[test]
fn issue2219_backquote_comsub_empty_operand_is_null_directory() {
    let (stdout, stderr, _code) = rubash("cd /tmp; x=`cd \"\" && pwd`; echo \"x=[$x] rc=$?\"");
    assert_eq!(stdout, "x=[] rc=1\n");
    assert!(stderr.contains("cd: null directory"), "stderr: {stderr}");
}

#[test]
fn issue2219_comsub_unquoted_split_expansion_declines_to_real_cd() {
    // e='a b' unquoted field-splits into two operands in the real
    // parse: GNU cd refuses with "too many arguments" (status 2).
    // The fast path must not flatten that into one bogus path probe.
    let (stdout, stderr, _code) =
        rubash("cd /tmp; e='a b'; x=$(cd $e && pwd); echo \"x=[$x] rc=$?\"");
    assert_eq!(stdout, "x=[] rc=2\n");
    assert!(
        stderr.contains("cd: too many arguments"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2219_controls_operand_less_and_normal_operand_unchanged() {
    let (stdout, stderr, _code) = rubash("x=$(cd && pwd); echo \"x=[$x] rc=$?\"");
    assert_eq!(stdout, format!("x=[{}] rc=0\n", home()));
    assert_eq!(stderr, "");

    let (stdout, stderr, _code) = rubash("x=$(cd /tmp && pwd); echo \"x=[$x] rc=$?\"");
    assert_eq!(stdout, "x=[/tmp] rc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2219_control_subshell_empty_operand_stays_null_directory() {
    // The real-executor face was already correct; pin the boundary so
    // the fast-path fix cannot drift it.
    let (stdout, stderr, _code) = rubash("cd /tmp; (cd \"\" && pwd); echo \"rc=$?\"");
    assert_eq!(stdout, "rc=1\n");
    assert!(stderr.contains("cd: null directory"), "stderr: {stderr}");
}
