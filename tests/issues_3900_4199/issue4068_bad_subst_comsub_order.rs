//! rubash#4068 — bad-substitution pre-scan discard must not skip
//! word-internal (or preceding-word) command substitution side effects.
//!
//! GNU subst.c:11531-11543 (`expand_word_internal`) -> `param_expand`
//! (`subst.c:10800`, `10832`) -> `parameter_brace_expand`
//! (`subst.c:10053`, `10276-10288`): brace expansion runs first
//! (`subst.c:13235`), then each word and each `$` expansion inside a word
//! is evaluated strictly left-to-right; command substitutions preceding
//! the failing `${...}` execute before `parameter_brace_expand` reaches
//! `bad_substitution:`, while anything after the failing `${...}` is
//! abandoned via `&expand_wdesc_error` -> `&expand_word_error`.
//!
//! Verified against WSL GNU Bash 5.3.x (`/usr/local/bin/bash`) via
//! script-file probes.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash4068-{tag}-{}", std::process::id()));
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
fn brace_and_comsub_before_bad_subst_runs_once_and_names_first_brace_product() {
    let script = concat!("echo {a,b}$(echo hi >&2)${-x}\n", "echo rc=$?\n",);
    let (stdout, stderr, code) = rubash_file("brace-comsub", script);
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "hi\nt.sh: line 1: a$(echo hi 1>&2)${-x}: bad substitution\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn empty_indirect_after_comsub_runs_comsub_and_succeeds() {
    let script = concat!("echo $(echo hi >&2)${!}\n", "echo rc=$?\n",);
    let (stdout, stderr, code) = rubash_file("empty-indir", script);
    assert_eq!(stdout, "\nrc=0\n");
    assert_eq!(stderr, "hi\n");
    assert_eq!(code, Some(0));
}

#[test]
fn comsub_after_bad_subst_never_runs() {
    let script = concat!("echo ${-x}$(echo nope >&2)\n", "echo rc=$?\n",);
    let (stdout, stderr, code) = rubash_file("comsub-after", script);
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: ${-x}$(echo nope 1>&2): bad substitution\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn assignment_rhs_comsub_before_bad_subst_runs_and_reports_whole_rhs() {
    let script = concat!("x=keep\n", "x=$(echo hi >&2)${-x}\n", "echo rc=$? x=$x\n",);
    let (stdout, stderr, code) = rubash_file("assign-before", script);
    assert_eq!(stdout, "rc=1 x=keep\n");
    assert_eq!(
        stderr,
        "hi\nt.sh: line 2: $(echo hi 1>&2)${-x}: bad substitution\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn assignment_rhs_comsub_after_bad_subst_never_runs() {
    let script = concat!("x=${-x}$(echo nope >&2)\n", "echo rc=$?\n",);
    let (stdout, stderr, code) = rubash_file("assign-after", script);
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: ${-x}$(echo nope 1>&2): bad substitution\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn multi_word_comsub_order_around_bad_subst() {
    let script = concat!(
        "echo $(echo first >&2) ${-x} $(echo second >&2)\n",
        "echo rc=$?\n",
    );
    let (stdout, stderr, code) = rubash_file("multi-word", script);
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, "first\nt.sh: line 1: ${-x}: bad substitution\n");
    assert_eq!(code, Some(0));
}

#[test]
fn backtick_before_bad_subst_in_double_quotes_runs() {
    let script = concat!("echo \"`echo hi >&2`${-x}\"\n", "echo rc=$?\n",);
    let (stdout, stderr, code) = rubash_file("backtick", script);
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "hi\nt.sh: line 1: `echo hi >&2`${-x}: bad substitution\n"
    );
    assert_eq!(code, Some(0));
}
