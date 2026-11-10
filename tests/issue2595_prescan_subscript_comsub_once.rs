//! rubash#2595 — the command-level error pre-scan's `:?` arm takes
//! its verdict from the EVALUATING fetch (`parameter_error_value`,
//! src/executor/parameter_errors.rs), so a command substitution
//! inside the tested subscript EXECUTES during the scan — and then
//! executes AGAIN when the real expansion fetches the element:
//! `a=(x y); : ${a[$(echo HIT >&2; echo 0)]:?err}` prints HIT twice.
//!
//! GNU law: there is no second fetch to hide behind.
//! parameter_brace_expand (subst.c:9777) fetches the value exactly
//! once — the array subscript is evaluated inside that single fetch
//! (parameter_brace_expand_word, subst.c:7663; the element read at
//! subst.c:10093) — and a `:?` verdict is taken from the fetched
//! value (subst.c:10347 operator dispatch; the error arm at
//! subst.c:10410-10418 calls parameter_brace_expand_error,
//! subst.c:8216, only when the fetched value is unset/null). The
//! rubash#2558 isolation (deferred subscript WRITES rolled back on a
//! no-error verdict) cannot cover this: a comsub's side effects do
//! not travel the deferred-write queue. The fix must give the scan a
//! non-executing verdict for comsub subscripts (defer to the
//! expander's own arm) or carry the scan's fetched value into the
//! real expansion — see fix-E1 REPORT.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings and
//! script files; HIT counts are execution counts — no diagnostic is
//! printed on the non-error shapes, so stderr is exactly the
//! comsub's own output).

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
fn issue2595_colon_form_comsub_subscript_runs_once() {
    let (stdout, stderr, code) =
        rubash("a=(x y); : ${a[$(echo HIT >&2; echo 0)]:?err}; echo \"rc=$? done\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 done\n");
    assert_eq!(stderr, "HIT\n", "comsub must execute exactly once");
}

#[test]
fn issue2595_value_form_comsub_subscript_runs_once() {
    // The fetched element is a[1]: a scan fetch that already ran the
    // comsub is harmless here (the subscript is a literal producer),
    // but the execution count is the defect — exactly one HIT.
    let (stdout, stderr, code) = rubash("a=(x y); echo \"val=${a[$(echo HIT >&2; echo 1)]:?err}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "val=y\n");
    assert_eq!(stderr, "HIT\n", "comsub must execute exactly once");
}

#[test]
fn issue2595_error_path_comsub_runs_once_and_stays_fatal() {
    // Element 5 is unset: GNU runs the comsub once (HIT), reports
    // `a[$(...)]: err` via parameter_brace_expand_error and the
    // non-interactive shell dies (expand_wdesc_fatal,
    // subst.c:10416-10418). The marker is produced by printf so the
    // count cannot be confused with the subscript text a diagnostic
    // may quote: the source never contains the literal `HIT`.
    let (stdout, stderr, code) =
        rubash("a=(x); : ${a[$(printf 'H%sT' I >&2; echo 5)]:?err}; echo alive");
    assert_ne!(code, Some(0), "the :? error must stay fatal");
    assert_eq!(stdout, "", "the command must abort, got: {stdout}");
    assert_eq!(
        stderr.matches("HIT").count(),
        1,
        "comsub must execute exactly once on the error path too: {stderr}"
    );
    assert!(stderr.contains("err"), "diagnostic expected: {stderr}");
}

#[test]
fn issue2595_scalar_subscript_comsub_runs_once() {
    let (stdout, stderr, code) =
        rubash("s=hi; : ${s[$(echo HIT >&2; echo 0)]:?err}; echo \"scalar ok\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "scalar ok\n");
    assert_eq!(stderr, "HIT\n", "comsub must execute exactly once");
}

#[test]
fn issue2595_chained_fragments_each_run_once() {
    // Chain: the array bound by the first statement feeds both
    // fragments; each fragment's comsub runs exactly once, in order.
    let (stdout, stderr, code) = rubash(
        "a=(x y); : ${a[$(echo HIT >&2; echo 0)]:?err}; echo \"mid=${a[$(echo HIT2 >&2; echo 1)]:?err}\"",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "mid=y\n");
    assert_eq!(stderr, "HIT\nHIT2\n");
}

#[test]
fn issue2595_default_operator_comsub_subscript_runs_once() {
    // Control family: `:-` decides from the same fetch. GNU runs the
    // comsub once and yields the element (a[1]=y), not the default.
    let (stdout, stderr, code) =
        rubash("a=(x y); echo \"val=${a[$(echo HIT >&2; echo 1)]:-dflt}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "val=y\n");
    assert_eq!(stderr, "HIT\n", "comsub must execute exactly once");
}

#[test]
fn issue2595_alternate_operator_comsub_subscript_runs_once() {
    // Control family: `:+` must evaluate the subscript to learn the
    // element is set — once — and yield the alternate word.
    let (stdout, stderr, code) = rubash("a=(x y); echo \"val=${a[$(echo HIT >&2; echo 1)]:+alt}\"");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "val=alt\n");
    assert_eq!(stderr, "HIT\n", "comsub must execute exactly once");
}
