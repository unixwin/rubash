//! rubash#1563 — the CMD_IGNORE_RETURN clearing frame for eval/source,
//! and the parse_and_execute DISCARD fold/containment rules.
//!
//! GNU execute_cmd.c:5004-5016 (execute_builtin): an eval or source
//! builtin dispatched under CMD_IGNORE_RETURN (`||`/`&&` tail, `!`,
//! condition — any position where errexit is suppressed for the
//! command) runs its body with exit_immediately_on_error cleared and
//! builtin_ignoring_errexit raised; error.c report_error therefore
//! only PRINTS a DISCARD-class diagnostic inside the body instead of
//! exit_shell-ing, and flags.c:262 keeps a `set -e` inside the body
//! from re-arming the print-time exit. Outside such a frame, a
//! DISCARD inside an eval/source body is caught by that body's own
//! parse_and_execute catch (builtins/evalstring.c:423-450), which
//! folds the pre-set EX_BADUSAGE 2 to EXECUTION_FAILURE 1; in a
//! subshell environment the same arm re-jumps, abandoning the
//! enclosing body with the folded status. A `( )` region installs
//! execute_in_subshell's preserving catch instead
//! (execute_cmd.c:1821-1825), so a failure raised directly inside a
//! region keeps status 2 even when the region sits in an eval body.
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
    let dir = std::env::temp_dir().join(format!("rubash-1563-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `script` as ./case.sh inside a fresh scratch dir (optionally
/// with extra files, e.g. a sourced p.sh), returning
/// (stdout, stderr, exit code).
fn run_script(script: &str, extra: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let dir = scratch("script");
    std::fs::write(dir.join("case.sh"), script).expect("write script");
    for (name, body) in extra {
        std::fs::write(dir.join(name), body).expect("write extra file");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("./case.sh")
        .current_dir(&dir)
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
fn issue_1563_eval_in_or_tail_survives_param_assign_discard() {
    // `eval ': ${v:=x}' || true` under -e: the diagnostic prints, the
    // eval body does not exit the shell.
    let (stdout, stderr, code) = run_script(
        "set -e\nreadonly v\neval ': ${v:=x}' || true\necho REACHED\n",
        &[],
    );
    assert_eq!(stdout, "REACHED\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
}

#[test]
fn issue_1563_eval_of_function_in_or_tail_survives() {
    let (stdout, stderr, code) = run_script(
        "set -e\nreadonly v\nf() { : ${v:=x}; }\neval 'f' || true\necho REACHED\n",
        &[],
    );
    assert_eq!(stdout, "REACHED\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
}

#[test]
fn issue_1563_source_in_or_tail_survives() {
    let (stdout, stderr, code) = run_script(
        "set -e\nreadonly v\n. ./p.sh || true\necho REACHED\n",
        &[("p.sh", ": ${v:=x}\n")],
    );
    assert_eq!(stdout, "REACHED\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
}

#[test]
fn issue_1563_framed_eval_inside_group_continues_the_group() {
    // The frame belongs to the eval command's own dispatch position
    // (an exempt element of the current-shell group), not to the
    // enclosing list: the rest of the group still runs.
    let (stdout, stderr, code) = run_script(
        "set -e\nreadonly v\n{ eval ': ${v:=x}' || true; echo IN-G; }\necho OUT\n",
        &[],
    );
    assert_eq!(stdout, "IN-G\nOUT\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
}

#[test]
fn issue_1563_no_frame_inside_subshell_region() {
    // Inside `( )` the eval catch re-jumps (subshell environment):
    // the region dies with the folded status 1, IN-SUB never runs.
    let (stdout, stderr, code) = run_script(
        "readonly v\n( eval ': ${v:=x}'; echo IN-SUB )\necho st=$?\n",
        &[],
    );
    assert_eq!(stdout, "st=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
    // Same for a sourced file inside a region.
    let (stdout, stderr, code) = run_script(
        "readonly v\n( . ./p.sh; echo IN-SUB )\necho st=$?\n",
        &[("p.sh", ": ${v:=x}\n")],
    );
    assert_eq!(stdout, "st=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
}

#[test]
fn issue_1563_eval_and_source_status_fold_to_one() {
    // No -e anywhere: the parse_and_execute catch folds the DISCARD
    // status to 1 for the eval/source command itself.
    let (stdout, stderr, code) = run_script("readonly v\neval ': ${v:=x}'\necho st=$?\n", &[]);
    assert_eq!(stdout, "st=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let (stdout, stderr, code) = run_script(
        "readonly v\n. ./p.sh\necho st=$?\n",
        &[("p.sh", ": ${v:=x}\n")],
    );
    assert_eq!(stdout, "st=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    // Special-parameter class inside an exempt eval folds the same way.
    let (stdout, stderr, code) =
        run_script("set -e\neval ': ${1:=x}' || true\necho REACHED\n", &[]);
    assert_eq!(stdout, "REACHED\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("cannot assign in this way").count(), 1);
}

#[test]
fn issue_1563_comsub_body_abandoned_by_eval_discard() {
    // eval inside a command-substitution body: the folded DISCARD
    // re-jumps, the whole body dies — the `|| true` and the echo after
    // it never run — and the assignment carries status 1, so the
    // outer -e exits 1 with no output.
    let (stdout, stderr, code) = run_script(
        "set -e\nreadonly v\nx=$(eval ': ${v:=x}' || true; echo IN-C)\necho \"OUT x=[$x]\"\n",
        &[],
    );
    assert_eq!(stdout, "", "stderr: {stderr}");
    assert_eq!(code, Some(1));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
}

#[test]
fn issue_1563_comsub_body_abandoned_by_source_discard() {
    let (stdout, stderr, code) = run_script(
        "readonly v\nx=$(. ./p.sh; echo INNER)\necho \"st=$? x=[$x]\"\n",
        &[("p.sh", ": ${v:=x}\n")],
    );
    assert_eq!(stdout, "st=1 x=[]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
    // And a plain DISCARD in the body folds to 1 as well.
    let (stdout, stderr, code) = run_script(
        "readonly v\nx=$(: ${v:=x}; echo INNER)\necho \"st=$? x=[$x]\"\n",
        &[],
    );
    assert_eq!(stdout, "st=1 x=[]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1563_frame_restores_errexit_on_exit() {
    // After the framed eval returns, errexit is live again: the next
    // bare failure exits.
    let (stdout, stderr, code) = run_script(
        "set -e\nreadonly v\neval ': ${v:=x}' || true\necho MID\nfalse\necho NEVER\n",
        &[],
    );
    assert_eq!(stdout, "MID\n", "stderr: {stderr}");
    assert_eq!(code, Some(1));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
}

#[test]
fn issue_1563_set_e_inside_frame_does_not_rearm_print_exit() {
    // flags.c:262: with builtin_ignoring_errexit raised, `set -e`
    // inside the body flips the option but cannot re-arm
    // exit_immediately_on_error — the diagnostic still only prints.
    let (stdout, stderr, code) = run_script(
        "set -e\nreadonly v\neval 'set -e; : ${v:=x}' || true\necho AFTER\n",
        &[],
    );
    assert_eq!(stdout, "AFTER\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
}

#[test]
fn issue_1563_unframed_eval_folds_and_set_plus_e_persists() {
    // A non-exempt eval gets no frame, but its catch still folds:
    // eval returns 1; the `set +e` it ran persists, so the trailing
    // `false` does not exit either.
    let (stdout, stderr, code) = run_script(
        "set -e\nreadonly v\neval 'set +e; : ${v:=x}'\necho \"AFTER st=$?\"\nfalse\necho STILL\n",
        &[],
    );
    assert_eq!(stdout, "AFTER st=1\nSTILL\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
}

#[test]
fn issue_1563_subshell_region_inside_eval_body_preserves_status() {
    // The `( )` catch preserves the pre-set status 2 — the eval
    // body's fold must not reach inside the region.
    let (stdout, stderr, code) = run_script("readonly v\neval '( : ${v:=x} ); echo st=$?'\n", &[]);
    assert_eq!(stdout, "st=2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
}

#[test]
fn issue_1563_pipeline_element_inside_eval_body_folds() {
    // A pipeline element inherits the enclosing parse_and_execute
    // catch (the element child installs no catch of its own for a
    // simple command), so the last element's DISCARD folds to 1.
    let (stdout, stderr, code) =
        run_script("readonly v\neval 'true | : ${v:=x}'\necho st=$?\n", &[]);
    assert_eq!(stdout, "st=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(stderr.matches("v: readonly variable").count(), 1);
    // At top level the same element preserves 2 (reader catch).
    let (stdout, stderr, code) = run_script("readonly v\ntrue | : ${v:=x}\necho st=$?\n", &[]);
    assert_eq!(stdout, "st=2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}
