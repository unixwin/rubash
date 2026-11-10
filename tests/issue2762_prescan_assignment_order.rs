//! rubash#2762 — the parameter-error pre-scan is not order-aware:
//! it judges a `:?` fragment against the environment as it stood
//! BEFORE the command's words ran, so an earlier `${a:=ok}` in the
//! same word list (which GNU's in-order expansion has already
//! bound) does not protect a later `${a:?}`:
//! `echo ${a:=ok} ${a:?}` falsely aborts in rubash; GNU prints
//! `ok ok`. The heredoc-body scan has the same shape (a binding on
//! body line 1 must be visible to a check on line 2).
//!
//! GNU law: expansion is one left-to-right walk —
//! expand_word_list_internal (subst.c:13219) drives
//! expand_word_internal (subst.c:11229) per word in order, and
//! parameter_brace_expand's `=`/`:=` arm BINDS as it goes
//! (parameter_brace_expand_rhs, subst.c:7966), so a later fragment
//! of the same list — or a later line of the same heredoc body,
//! which GNU expands in one pass for the command — is adjudicated
//! against the evolved state. The rubash pre-scan
//! (src/executor/parameter_errors.rs: the `:?` arm reads
//! `parameter_error_value` against the live pre-expansion
//! environment; the assignment arms check for assignment ERRORS
//! but never record the binding) simulates every fragment against
//! the same snapshot. The fix must thread the scan's simulated
//! bindings in order (or fold the check into the in-order walk) —
//! NOT special-case `:=` before `:?`: the control cells below pin
//! that `:-` does not bind, that colon-less `=` does not rebind a
//! null-but-set parameter, and that a genuinely earlier `:?` still
//! fires first.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings and
//! script files).

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
fn issue2762_assign_then_error_check_two_words() {
    let (stdout, stderr, code) = rubash("echo ${a:=ok} ${a:?}");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ok ok\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2762_assign_then_error_check_with_message() {
    let (stdout, stderr, code) = rubash("echo ${a:=ok} ${a:?boom}");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ok ok\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2762_assign_then_error_check_same_word() {
    let (stdout, stderr, code) = rubash("echo ${a:=ok}${a:?}");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "okok\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2762_assign_then_error_check_in_assignment_value() {
    // Chain: the := binding lands inside the assignment's own value
    // expansion and the following fragment of the same value reads
    // it; the assigned variable then carries both expansions.
    let (stdout, stderr, code) = rubash("x=${a:=ok}${a:?}; echo \"x=$x\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "x=okok\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2762_colonless_assign_binds_when_unset() {
    let (stdout, stderr, code) = rubash("echo ${a=ok} ${a:?}");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ok ok\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2762_colon_assign_rebinds_null() {
    let (stdout, stderr, code) = rubash("a=; echo ${a:=ok} ${a:?}");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ok ok\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2762_heredoc_cat_assign_then_check() {
    let (stdout, stderr, code) = rubash("cat <<EOF\n${a:=ok}\n${a:?}\nEOF");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ok\nok\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2762_heredoc_colon_assign_then_check() {
    // The issue's own heredoc shape: the body is expanded for the
    // `:` command; a line-1 binding must satisfy the line-2 check
    // and the script survives.
    let (stdout, stderr, code) = rubash(": <<EOF\n${a:=ok}\n${a:?}\nEOF\necho survived");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "survived\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2762_cross_command_assignment_still_visible() {
    // Control (chain): across commands the binding is real state,
    // not scan simulation — this must pass before and after.
    let (stdout, stderr, code) = rubash("echo ${a:=ok}; echo ${a:?}");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ok\nok\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2762_error_check_before_assign_still_fires() {
    // Control: order cuts both ways — a :? that genuinely comes
    // first sees the unbound parameter and aborts before the :=
    // fragment is reached.
    let (stdout, stderr, code) = rubash("echo ${a:?} ${a:=ok}; echo alive5");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "", "the command must abort, got: {stdout}");
    assert!(
        stderr.contains("a: parameter null or not set"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2762_default_operator_does_not_bind() {
    // Control: `:-` substitutes without binding, so the later :?
    // still fires. A fix that treats any earlier operator as a
    // binding fails this cell.
    let (stdout, stderr, code) = rubash("echo ${a:-x} ${a:?}; echo alive6");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "", "the command must abort, got: {stdout}");
    assert!(
        stderr.contains("a: parameter null or not set"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2762_colonless_assign_does_not_rebind_null() {
    // Control: `a` is set but null; colon-less `=` does not rebind
    // it, so the colon form :? still fires.
    let (stdout, stderr, code) = rubash("a=; echo ${a=ok} ${a:?}; echo alive7");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "", "the command must abort, got: {stdout}");
    assert!(
        stderr.contains("a: parameter null or not set"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2762_heredoc_colon_check_before_assign_still_fires() {
    // Control (heredoc, special builtin): the check precedes the
    // binding inside the same body, so the error fires and — the
    // body belongs to `:` — the script aborts.
    let (stdout, stderr, code) = rubash(": <<EOF\n${a:?}\n${a:=ok}\nEOF\necho after");
    assert_ne!(code, Some(0));
    assert_eq!(stdout, "", "the script must abort, got: {stdout}");
    assert!(
        stderr.contains("a: parameter null or not set"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue2762_heredoc_cat_check_before_assign_fails_command_only() {
    // Control (heredoc, ordinary builtin): GNU reports the error,
    // the heredoc expansion aborts so `cat` never runs, but the
    // script continues — an expansion error in a redirection body
    // is a command failure, not a script fatality (measured on GNU
    // 5.3.20; the `:` form above is the fatal contrast).
    let (stdout, stderr, code) = rubash("cat <<EOF\n${a:?}\n${a:=ok}\nEOF\necho after");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "after\n");
    assert!(
        stderr.contains("a: parameter null or not set"),
        "stderr: {stderr}"
    );
}
