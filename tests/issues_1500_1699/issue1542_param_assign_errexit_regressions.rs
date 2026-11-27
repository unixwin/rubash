//! rubash#1542 — a failed `${var:=word}` / `${var=word}` bind against
//! a readonly (or special/positional) target and its status rules.
//!
//! GNU prints the readonly diagnostic from inside bind_variable
//! (variables.c err_readonly -> error.c report_error), BEFORE
//! subst.c's parameter_brace_expand_rhs writes EX_BADUSAGE into
//! last_command_exit_value; the special-parameter arm
//! (subst.c:10402-10404) sets EXECUTION_FAILURE and then calls the
//! same report_error. report_error exit_shells on the spot when
//! exit_immediately_on_error is live, with the status current at
//! print time: the pre-expansion `$?` (0 remapped to
//! EXECUTION_FAILURE) for the readonly class, 1 for the special
//! class. The flag is the raw option — errexit-exempt positions
//! (`||`, `&&`, `!`, if conditions, function bodies called there)
//! do NOT clear it, so they all exit; no ERR trap fires. Without
//! errexit the failure is a plain DISCARD: the command list is
//! abandoned with the pre-set status (2 readonly / 1 special) and
//! the script continues — folded to 1 inside a command-substitution
//! body like every DISCARD in that frame (rubash#1541).
//!
//! Expectations measured against GNU bash 5.3.20 with script files
//! in this environment (byte-level stdout, diagnostic counts,
//! statuses; each form run 3x with identical results).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-1542-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("script");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn issue_1542_readonly_assign_expansion_without_errexit_is_status_2() {
    let (stdout, stderr, code) = run_script("readonly v\necho ${v:=x}\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("v: readonly variable").count(),
        1,
        "stderr: {stderr}"
    );
    // The `=` (unset-only) operator fails the same way.
    let (stdout, stderr, code) = run_script("readonly v\necho ${v=x}\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("v: readonly variable").count(),
        1,
        "stderr: {stderr}"
    );
    // Assignment-statement form.
    let (stdout, stderr, code) = run_script("readonly v\nFOO=${v:=x}\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1542_errexit_exits_with_pre_expansion_status() {
    // Previous status 0 -> report_error remaps to EXECUTION_FAILURE.
    let (stdout, stderr, code) = run_script("set -e\nreadonly v\necho ${v:=x}\necho never\n");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(code, Some(1));
    assert_eq!(
        stderr.matches("v: readonly variable").count(),
        1,
        "stderr: {stderr}"
    );
    // Previous status 5 (left by the if condition) -> exits 5.
    let (stdout, stderr, code) = run_script(
        "set -e\nreadonly v\nif (exit 5); then echo a; else echo ${v:=x}; fi\necho after\n",
    );
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(code, Some(5));
    assert_eq!(
        stderr.matches("v: readonly variable").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1542_errexit_exempt_positions_still_exit() {
    // `||` position: report_error sees the raw flag, so "handled"
    // never prints and the shell dies at the diagnostic.
    let (stdout, stderr, code) =
        run_script("set -e\nreadonly v\n: ${v:=x} || echo \"handled rc=$?\"\necho after\n");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(code, Some(1));
    // if-condition position.
    let (stdout, stderr, code) =
        run_script("set -e\nreadonly v\nif echo ${v:=x}; then echo then; fi\necho after\n");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(code, Some(1));
    // Prefix-assignment form.
    let (stdout, stderr, code) =
        run_script("set -e\nreadonly v\nFOO=${v:=x} echo hi\necho after\n");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn issue_1542_errexit_follows_the_current_flag_state() {
    // Fatality tracks the flag at diagnostic time: cleared by then,
    // the failure is the plain status-2 DISCARD and the script runs on.
    let (stdout, stderr, code) =
        run_script("set -e\nset +e\nreadonly v\necho ${v:=x}\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1542_comsub_body_folds_the_discard_to_1() {
    let (stdout, stderr, code) = run_script("readonly v\nx=$(echo ${v:=x})\necho \"s=$?\"\n");
    assert_eq!(stdout, "s=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("v: readonly variable").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1542_pipeline_element_statuses() {
    // Without errexit the element dies with the pre-set 2 and the
    // script continues (the failure must not leak into the parent).
    let (stdout, stderr, code) =
        run_script("readonly v\necho ${v:=x} | cat\necho \"after ps=${PIPESTATUS[0]}\"\n");
    assert_eq!(stdout, "after ps=2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("v: readonly variable").count(),
        1,
        "stderr: {stderr}"
    );
    // Under errexit the element child keeps the flag: report_error
    // exits it with the pre-bind status (1); the pipeline's own
    // status is cat's 0, so the script itself survives.
    let (stdout, stderr, code) =
        run_script("set -e\nreadonly v\necho ${v:=x} | cat\necho \"after ps=${PIPESTATUS[0]}\"\n");
    assert_eq!(stdout, "after ps=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("v: readonly variable").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1542_colon_question_class_stays_fatal() {
    // `${var:?}` is the separate FORCE_EOF class (subst.c:
    // parameter_brace_expand_error): unchanged by this fix — the
    // noninteractive shell exits 1 and the next line never runs.
    let (stdout, stderr, code) = run_script("echo ${undef:?boom}\necho after\n");
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(code, Some(1));
    assert!(stderr.contains("undef: boom"), "stderr: {stderr}");
}
