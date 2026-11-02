//! rubash#2558 — the command-level error pre-scan's `:?` arm ran the
//! EVALUATING fetch (`parameter_error_value`) for the operator's
//! left-hand side. For a SET subscripted parameter the arm returns no
//! verdict — but the subscript's side effects already ran — and the
//! real expansion then fetched (and evaluated) a second time:
//! `a=(x y); i=0; : ${a[i++]:?err}` left i=2 where GNU leaves i=1.
//!
//! GNU law: parameter_brace_expand (subst.c:9777) fetches exactly
//! once — array_value_internal evaluates the subscript inside that
//! single fetch, and the message word is expanded only on the error
//! path (parameter_brace_expand_error, subst.c:8216-8230). An earlier
//! prescribed fix — letting the arm skip fragments whose subscript
//! the non-evaluating probe cannot settle — was implemented and
//! refuted: the scan is the sole `:?` adjudicator (the expander's
//! arm never errors), so skipping turned proven fatal shapes
//! (`${b[i]:?err}` and friends) into silent rc=0. The shipped fix
//! instead isolates the scan's deferred subscript writes: the scan
//! keeps its evaluating fetch and its verdicts, but when it reaches
//! no error verdict the writes its evaluations queued are rolled
//! back, so the real expansion evaluates each subscript exactly
//! once; on an error verdict the queue is untouched, because the
//! command aborts and the scan's evaluation is the only one.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings).

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
fn issue2558_set_element_subscript_evaluated_once() {
    let (stdout, stderr, code) = rubash("a=(x y); i=0; : ${a[i++]:?err}; echo \"i=$i\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "i=1\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2558_two_fragments_each_evaluated_once() {
    let (stdout, stderr, code) =
        rubash("a=(x y); i=0; : ${a[i++]:?err} ${a[i++]:?err2}; echo \"i=$i\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "i=2\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2558_message_word_not_scanned_when_parameter_set() {
    // The outer parameter is set, so GNU never expands the message
    // word: the nested fragment's subscript must not run at either
    // layer, and the outer subscript runs exactly once.
    let (stdout, stderr, code) =
        rubash("a=(x y); i=0; j=0; : ${a[i++]:?${m[j++]:?inner}}; echo \"i=$i j=$j\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "i=1 j=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2558_value_form_reads_the_fetched_element() {
    // GNU: the single fetch reads a[0]; a scan fetch that already
    // advanced the subscript makes the real fetch return a[1].
    let (stdout, _stderr, code) = rubash("a=(x y); i=0; echo \"val=${a[i++]:?err} i=$i\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "val=x i=1\n");
}

#[test]
fn issue2558_sparse_second_fragment_still_errors() {
    // Intra-scan sequencing is load-bearing: the second fragment's
    // verdict must observe the first fragment's evaluation (a[1] is
    // unset in a one-element array), so the error path is unchanged.
    let (stdout, stderr, code) = rubash("a=(x); i=0; : ${a[i++]:?e} ${a[i++]:?e}; echo alive");
    assert!(
        stderr.contains("a[i++]: e"),
        "expected the second fragment to raise its :? error, got: {stderr}"
    );
    assert_ne!(code, Some(0));
    assert!(
        !stdout.contains("alive"),
        "command must abort, got: {stdout}"
    );
}

#[test]
fn issue2558_mixed_operator_fragments_each_evaluate_once() {
    let (stdout, _stderr, code) =
        rubash("a=(x); i=0; : ${a[i++]:?e} ${a[i++]:-d}; echo \"i=$i alive\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "i=2 alive\n");
}

#[test]
fn issue2558_unset_element_errors_after_one_evaluation() {
    // Control: an UNSET element still errors (the subshell dies on
    // the `:?`), its diagnostic names the parameter as written, and
    // the subscript has been evaluated exactly once when it does —
    // observed through the subshell's EXIT trap (i starts at 5).
    let (stdout, stderr, code) =
        rubash("( i=5; trap 'echo \"i=$i\"' EXIT; : ${b[i++]:?err} ); echo \"outer done\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "i=6\nouter done\n");
    assert!(stderr.contains("b[i++]: err"), "stderr: {stderr}");
}
