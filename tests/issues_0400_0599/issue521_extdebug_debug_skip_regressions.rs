//! Issue rubash#521 — extdebug DEBUG-skip must be honored at the compound
//! fire sites.
//!
//! GNU's fire-site contract (execute_cmd.c:3057 for-head, :3191 arith-for
//! expressions, :3532 select-head, :3672 case-head, :4510 simple command,
//! :5391 function entry) is `debugging_mode && retval != EXECUTION_SUCCESS`
//! — skip only under extdebug, on ANY non-zero trap status. Rubash's old
//! rule skipped only on status 2 with no extdebug gate, and computed the
//! decision then ignored it at the compound-head/pipeline-stage/function-
//! entry fire sites.
//!
//! Per-site skip semantics (probe-verified against WSL GNU Bash 5.3.0,
//! one-shot trap scripts k1/k2/k8/k9/m1-m3, 2026-10-12):
//! - case/select head: the whole command is skipped, status EXECUTION_
//!   SUCCESS (k9: `CS=0` with the body absent);
//! - function entry: the body and the RETURN trap are skipped, the call's
//!   status is 0 (k8/m2: `GN=0`/`FN=0` with the body's echo absent);
//! - for header: the iteration's body is skipped, the loop proceeds (m3:
//!   `B1` skipped, `B2` ran); a skip on the final iteration leaves the
//!   for command's status at the trap's (k1: `FOR=1`);
//! - arith-for init/test/update: the expression evaluates as 0 with ok=1
//!   (k2: the skipped update left `i` unchanged → `A0 A0 A1 A2`);
//! - pipeline stage: the skipped stage produces no output and its status
//!   is the trap's (i521d: both stages skipped → `pipe-R=1`... with an
//!   always-failing trap the trailing echo skips too, so the observable
//!   is empty output like GNU);
//! - $? is preserved across a DEBUG firing (`trap 'false' DEBUG; true;
//!   echo $?` prints 0 — z1 probe); without extdebug nothing ever skips.

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

/// The issue repro: the DEBUG trap returns 1 on its second firing — the
/// case HEAD — and the case body must not run (GNU: `R=0` only).
#[test]
fn case_head_skip_under_extdebug() {
    let (stdout, _stderr, code) = rubash(
        "shopt -s extdebug; n=0; tr(){ n=$((n+1)); [[ $n == 2 ]] && return 1; return 0; }; \
         trap tr DEBUG; false; case a in a) echo m;; esac; echo \"R=$?\"",
    );
    assert_eq!(stdout, "R=0\n");
    assert_eq!(code, Some(0));
}

/// One-shot trap at the case head (probe k9): the case is skipped, its
/// status is 0, and the next command runs.
#[test]
fn case_skip_leaves_success_status() {
    let (stdout, _stderr, _code) = rubash(
        "shopt -s extdebug; skip=0; \
         tr(){ if [[ $skip == 1 ]]; then skip=2; return 1; fi; return 0; }; \
         trap tr DEBUG; case a in a) echo C1;; esac; \
         skip=1; case b in b) echo C2;; esac; echo \"CS=$?\"",
    );
    assert_eq!(stdout, "C1\nCS=0\n");
}

/// Function-entry skip: the body never runs, the call's status is 0 (m2).
#[test]
fn function_entry_skip_skips_body_with_success_status() {
    let (stdout, _stderr, _code) = rubash(
        "shopt -s extdebug; n=0; \
         tr(){ n=$((n+1)); [[ $n == 2 ]] && return 1; return 0; }; \
         trap tr DEBUG; g(){ echo G1; }; false; g; echo \"FN=$?\"",
    );
    assert_eq!(stdout, "FN=0\n");
}

/// For-header skip: the iteration body is skipped, the loop continues
/// (m3: B1 skipped, B2 ran, FOR=0).
#[test]
fn for_header_skip_skips_only_that_iteration() {
    let (stdout, _stderr, _code) = rubash(
        "shopt -s extdebug; n=0; \
         tr(){ n=$((n+1)); [[ $n == 2 ]] && return 1; return 0; }; \
         trap tr DEBUG; false; for i in 1 2; do echo B$i; done; echo \"FOR=$?\"",
    );
    assert_eq!(stdout, "B2\nFOR=0\n");
}

/// For-header skip on the FINAL iteration leaves the trap's status (k1).
#[test]
fn for_header_skip_on_final_iteration_keeps_trap_status() {
    let (stdout, _stderr, _code) = rubash(
        "shopt -s extdebug; skip=0; \
         tr(){ if [[ $skip == 1 ]]; then skip=2; return 1; fi; return 0; }; \
         trap tr DEBUG; for i in 1 2; do echo \"B$i\"; skip=1; done; echo \"FOR=$?\"",
    );
    assert_eq!(stdout, "B1\nFOR=1\n");
}

/// Arith-for update skip: the expression evaluates as 0, `i` keeps its
/// value, the loop re-runs the same index (k2: A0 A0 A1 A2, AF=1 i=3).
#[test]
fn arith_for_update_skip_evaluates_zero() {
    let (stdout, _stderr, _code) = rubash(
        "shopt -s extdebug; skip=0; \
         tr(){ if [[ $skip == 1 ]]; then skip=2; return 1; fi; return 0; }; \
         trap tr DEBUG; \
         for ((i=0;i<3;i++)); do echo \"A$i\"; [[ $skip == 0 ]] && skip=1; done; \
         echo \"AF=$? i=$i\"",
    );
    assert_eq!(stdout, "A0\nA0\nA1\nA2\nAF=1 i=3\n");
}

/// Pipeline-stage skip: the skipped stage produces no output and its
/// status is the trap's (i521d — with an always-failing trap the trailing
/// echo skips too, so GNU's observable is empty output and rc 0).
#[test]
fn pipeline_stage_skip_produces_no_output() {
    let (stdout, _stderr, code) =
        rubash("shopt -s extdebug; trap 'false' DEBUG; echo A | echo B; echo \"pipe-R=$?\"");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(0));
}

/// Without extdebug nothing skips, whatever the trap's status, and $?
/// survives the firing (z1 probe: `trap 'false' DEBUG; true; echo $?`
/// prints 0).
#[test]
fn no_extdebug_no_skip_and_status_preserved() {
    let (stdout, _stderr, _code) = rubash("trap 'false' DEBUG; true; echo \"Q=$?\"");
    assert_eq!(stdout, "Q=0\n");
}

/// Guard: the old status-2 rule skipped without extdebug; GNU never does.
#[test]
fn status_two_without_extdebug_does_not_skip() {
    let (stdout, _stderr, _code) = rubash(
        "n=0; tr(){ n=$((n+1)); [[ $n == 2 ]] && return 2; return 0; }; \
         trap tr DEBUG; echo first; echo second; echo \"R=$?\"",
    );
    assert_eq!(stdout, "first\nsecond\nR=0\n");
}

/// Guard: simple-command skip under extdebug leaves $? = 0 (m1).
#[test]
fn simple_command_skip_leaves_success_status() {
    let (stdout, _stderr, _code) = rubash(
        "shopt -s extdebug; n=0; \
         tr(){ n=$((n+1)); [[ $n == 2 ]] && return 1; return 0; }; \
         trap tr DEBUG; false; echo hi; echo \"SIMPLE=$?\"",
    );
    assert_eq!(stdout, "SIMPLE=0\n");
}
