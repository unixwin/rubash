//! niubash#993 §1 — under `set -u`, a `${X:=word}` fragment's bind is
//! invisible to a later `$X` inside the SAME command:
//! `echo "a=${X:=v} b=$X"` died with `X: unbound variable` while GNU
//! prints `a=v b=v`.
//!
//! GNU law: expansion is one left-to-right walk —
//! expand_word_list_internal (subst.c:13219) drives
//! expand_word_internal (subst.c:11229) per word in order, and
//! parameter_brace_expand's `=`/`:=` arm BINDS as it goes
//! (parameter_brace_expand_rhs, subst.c:7966), so a later fragment of
//! the same word list is adjudicated against the evolved state. The
//! nounset pre-scan (parameter_errors.rs
//! nounset_unbound_parameter -> nounset_braced_parameter_is_unbound)
//! judged every `$name` reference against the pre-command environment
//! and never consulted the simulated-binding table rubash#2762 built
//! for the `:?` adjudication — so the plain-reference arm reported a
//! variable the in-order walk had already bound.
//!
//! The nounset walks now record each `:=`/`=` fragment they pass (the
//! same scan_record_assignment the `${...}` error walk uses) and the
//! boundness verdicts consult the simulated table. Ordering is kept
//! strictly left-to-right: the word-context nounset pass keeps its
//! records private (the `${...}` error walk re-records in order), the
//! simple-command pre-scan clears the table between its nounset pass
//! and the per-word error walk, and every scan entry that can run
//! outside a begun context (the admission probe, the `[[ =~ ]]`
//! operand scan) begins a fresh one.
//!
//! Expectations verified against GNU bash 5.3.20 (script files; the
//! `wsl bash -c` passthrough cannot carry `$` probes).

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
fn niubash993_s1_assign_then_read_same_word() {
    // The issue's headline repro.
    let (stdout, stderr, code) = rubash("set -u; echo \"a=${X:=v} b=$X\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "a=v b=v\n");
    assert_eq!(stderr, "");
}

#[test]
fn niubash993_s1_assign_then_read_later_word() {
    let (stdout, stderr, code) = rubash("set -u; echo \"${X:=v}\" \"$X\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "v v\n");
    assert_eq!(stderr, "");
}

#[test]
fn niubash993_s1_assign_inside_assignment_value() {
    let (stdout, stderr, code) = rubash("set -u; y=\"${X:=v}/$X\"; echo \"$y\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "v/v\n");
    assert_eq!(stderr, "");
}

#[test]
fn niubash993_s1_adjacent_fragments() {
    let (stdout, stderr, code) = rubash("set -u; echo \"${X:=v}${X}\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "vv\n");
    assert_eq!(stderr, "");
}

#[test]
fn niubash993_s1_assign_dynamic_word_then_read() {
    // The operator word's value is unknown to the scan (a command
    // substitution), but its set-ness is recorded all the same.
    let (stdout, stderr, code) = rubash("set -u; echo \"${X:=$(printf dyn)} mid $X\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "dyn mid dyn\n");
    assert_eq!(stderr, "");
}

#[test]
fn niubash993_s1_element_assign_then_read() {
    // GNU binds the element, so both the element and the bare scalar
    // read (element 0) are bound afterwards.
    let (stdout, stderr, code) = rubash("set -u; echo \"${a[0]:=el} ${a[0]} ${a}\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "el el el\n");
    assert_eq!(stderr, "");
}

#[test]
fn niubash993_s1_nested_operator_word_binds() {
    // The default word IS expanded for an unset subject, so a `:=`
    // inside it binds for the fragments after it.
    let (stdout, stderr, code) = rubash("set -u; echo \"${a:-${b:=x}} ${b}\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x x\n");
    assert_eq!(stderr, "");
}

#[test]
fn niubash993_s1_later_default_word_sees_bind() {
    // `:-` on a (simulated-)bound subject does not expand its word at
    // all — an unbound name inside it must not fire.
    let (stdout, stderr, code) = rubash("set -u; echo \"${X:=v} ${X:-$UNSET}\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "v v\n");
    assert_eq!(stderr, "");
}

#[test]
fn niubash993_s1_later_alternate_word_sees_bind() {
    // `:+` on a bound-nonempty subject expands its word — an unbound
    // name inside still fires.
    let (_stdout, stderr, code) = rubash("set -u; echo \"${X:=v} ${X:+$UNSET}\"");
    assert_ne!(code, Some(0));
    assert!(
        stderr.contains("UNSET: unbound variable"),
        "stderr: {stderr}"
    );
}

#[test]
fn niubash993_s1_read_before_assign_still_fires() {
    // Order cuts both ways: a reference that genuinely precedes the
    // binding still aborts before the `:=` fragment is reached.
    let (stdout, stderr, code) = rubash("set -u; echo \"$X ${X:=late}\"; echo alive");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "");
    assert!(stderr.contains("X: unbound variable"), "stderr: {stderr}");
}

#[test]
fn niubash993_s1_error_check_before_assign_still_fires() {
    // A `:?` earlier in the SAME word than a `:=` still fires — the
    // error walk must not see the later fragment's bind.
    let (stdout, stderr, code) = rubash("set -u; echo \"${X:?boom} ${X:=v}\"; echo alive");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "");
    assert!(stderr.contains("X: boom"), "stderr: {stderr}");
}

#[test]
fn niubash993_s1_error_check_later_word_before_assign_still_fires() {
    // Same ordering across words: word 1's `:?` precedes word 2's `:=`.
    let (stdout, stderr, code) = rubash("set -u; echo \"${X:?boom}\" \"${X:=v}\"; echo alive");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "");
    assert!(stderr.contains("X: boom"), "stderr: {stderr}");
}

#[test]
fn niubash993_s1_error_check_after_assign_passes() {
    // The rubash#2762 contract under `set -u` this time: a `:?` AFTER
    // the `:=` fragment sees the bind.
    let (stdout, stderr, code) = rubash("set -u; echo ${a:=ok} ${a:?}");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "ok ok\n");
    assert_eq!(stderr, "");
}

#[test]
fn niubash993_s1_default_operator_does_not_bind() {
    // `:-` substitutes without binding; the later read still fires.
    let (_stdout, stderr, code) = rubash("set -u; echo \"${X:-x} ${X}\"; echo alive");
    assert_ne!(code, Some(0));
    assert!(stderr.contains("X: unbound variable"), "stderr: {stderr}");
}

#[test]
fn niubash993_s1_colonless_assign_does_not_rebind_null() {
    // `=` on a set-but-null parameter does not bind, so the later `:?`
    // still fires (GNU agrees).
    let (_stdout, stderr, code) = rubash("set -u; a=; echo \"${a=word} ${a:?chk}\"; echo alive");
    assert_ne!(code, Some(0));
    assert!(stderr.contains("a: chk"), "stderr: {stderr}");
}

#[test]
fn niubash993_s1_colon_assign_rebinds_null() {
    // `:=` DOES rebind a set-but-null parameter — the later `:?` and
    // the read both see it.
    let (stdout, stderr, code) = rubash("set -u; a=; echo \"${a:=word} ${a:?chk} ${a}\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "word word word\n");
    assert_eq!(stderr, "");
}

#[test]
fn niubash993_s1_cross_command_real_state() {
    // Control: across commands the binding is real state, not scan
    // simulation — both before and after the fix.
    let (stdout, stderr, code) = rubash("set -u; echo \"${X:=v}\"; echo \"$X\"");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "v\nv\n");
    assert_eq!(stderr, "");
}
