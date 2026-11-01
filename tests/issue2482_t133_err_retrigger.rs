//! rubash#2482 (t133 residual): an exit-terminated command chain must
//! not re-trigger the ERR trap.
//!
//! GNU's `exit` builtin never returns a status to any caller frame:
//! builtins/exit.def `exit_or_logout` ends in
//! `jump_to_top_level (EXITBLTIN)`, and run_exit_trap (trap.c:1027)
//! installs its own setjmp(top_level) to catch it. The
//! run_error_trap call sites (execute_cmd.c:773/1000/1166/2909) only
//! ever adjudicate a command that COMPLETED, so no frame an exit
//! unwind crosses — main body, function chain, or trap action —
//! performs failure adjudication for the chain. Ordinary failures
//! inside trap actions adjudicate exactly like anywhere else, and an
//! `exit` inside a subshell (`( exit 1 )`, a pipeline element) is an
//! ordinary failure of that subshell command in the parent.
//!
//! rubash adjudicated one extra time: execute_materialized_command's
//! errexit tail fired maybe_run_error_trap for an in-flight
//! Err(ExitCode) — at the exit builtin's own dispatch frame, and
//! again at eval/source wrapper frames whose inner command had
//! already been adjudicated. This is the bats cell-133 residual: the
//! fixture's EXIT-teardown chain ends in `exit "$status"`, and the
//! extra fire re-ran bats_error_trap, printing a second raw
//! `failure callback` line (GNU prints one).
//!
//! Expectations are GNU bash 5.3.20 outputs, probed from script
//! files on 2026-11-02 (probes x3–x22 / y1–y10 / n2–n3 in the
//! cluster workdir).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn rubash_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT");
    command
}

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn test_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("rubash-issue2482t133-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_script(dir: &Path, name: &str, body: &str) -> std::process::Output {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script");
    rubash_command()
        .current_dir(dir)
        .arg(shell_test_path(&script))
        .output()
        .expect("run script")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn errfire_count(output: &std::process::Output) -> usize {
    stdout(output).matches("ERRFIRE").count()
}

// 01 — the t133 core shape: EXIT action is a function chain ending
// in `exit`. GNU fires the ERR trap zero times for the chain.
#[test]
fn issue2482_t133_01_exit_action_chain_no_fire() {
    let dir = test_dir("01");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         g() { exit 1; }\n\
         h() { g; }\n\
         trap \"h\" EXIT\n\
         true\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 0);
    let _ = fs::remove_dir_all(&dir);
}

// 02 — a real failure before the exit: the body's failure fires
// once; the EXIT action's exit-terminated chain adds no second
// fire. GNU: exactly 1.
#[test]
fn issue2482_t133_02_prior_failure_fires_once_only() {
    let dir = test_dir("02");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         g() { exit 1; }\n\
         h() { g; }\n\
         trap \"h\" EXIT\n\
         f() { false; }\n\
         f\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 1);
    assert!(!stdout(&output).contains("after"));
    let _ = fs::remove_dir_all(&dir);
}

// 03 — the minimal shape: the EXIT action is a bare `exit 3`.
// GNU fires zero times and the shell exits 3.
#[test]
fn issue2482_t133_03_bare_exit_in_action_no_fire() {
    let dir = test_dir("03");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         trap \"exit 3\" EXIT\n\
         true\n",
    );
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(errfire_count(&output), 0);
    let _ = fs::remove_dir_all(&dir);
}

// 04 — the same chain in the main body (not in any trap action):
// GNU fires zero times. The exemption is the exit unwind itself,
// not a trap-action gate.
#[test]
fn issue2482_t133_04_main_body_chain_no_fire() {
    let dir = test_dir("04");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         g() { exit 1; }\n\
         h() { g; }\n\
         h\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 0);
    assert!(!stdout(&output).contains("after"));
    let _ = fs::remove_dir_all(&dir);
}

// 05 — a bare `exit` as a plain top-level command under -eET.
// GNU fires zero times.
#[test]
fn issue2482_t133_05_bare_exit_no_fire() {
    let dir = test_dir("05");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         exit 1\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 0);
    let _ = fs::remove_dir_all(&dir);
}

// 06 — the exit unwind arriving through an `eval` wrapper: the
// wrapper frame must not adjudicate it either. GNU: zero fires.
#[test]
fn issue2482_t133_06_eval_exit_no_fire() {
    let dir = test_dir("06");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         eval 'exit 1'\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 0);
    assert!(!stdout(&output).contains("after"));
    let _ = fs::remove_dir_all(&dir);
}

// 07 — the wrapper face with an ordinary failure: `eval 'false'`
// is adjudicated exactly once, at the inner completion — the eval
// frame's own tail must not fire a second time. GNU: exactly 1.
#[test]
fn issue2482_t133_07_eval_false_fires_once() {
    let dir = test_dir("07");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         eval 'false'\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 1);
    assert!(!stdout(&output).contains("after"));
    let _ = fs::remove_dir_all(&dir);
}

// 08 — sourced file whose body exits: the source wrapper frame
// must not adjudicate the unwind. GNU: zero fires.
#[test]
fn issue2482_t133_08_source_exit_no_fire() {
    let dir = test_dir("08");
    fs::write(dir.join("src.sh"), "exit 1\n").expect("write sourced file");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         . ./src.sh\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 0);
    assert!(!stdout(&output).contains("after"));
    let _ = fs::remove_dir_all(&dir);
}

// 09 — sourced file whose body fails ordinarily: exactly one
// fire, at the inner completion. GNU: exactly 1.
#[test]
fn issue2482_t133_09_source_false_fires_once() {
    let dir = test_dir("09");
    fs::write(dir.join("src.sh"), "false\n").expect("write sourced file");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         . ./src.sh\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 1);
    assert!(!stdout(&output).contains("after"));
    let _ = fs::remove_dir_all(&dir);
}

// 10 — an `exit` inside a subshell is NOT the exempt shape: the
// subshell command completes with status 1 in the parent and is
// adjudicated once (and -e then aborts the action, so the
// function's continuation never prints). GNU: exactly 1.
#[test]
fn issue2482_t133_10_subshell_exit_in_action_function_fires_once() {
    let dir = test_dir("10");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         g() { ( exit 1 ); echo g-after; }\n\
         trap \"g\" EXIT\n\
         true\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 1);
    assert!(!stdout(&output).contains("g-after"));
    let _ = fs::remove_dir_all(&dir);
}

// 11 — a `( exit 1 )` statement at the top level: adjudicated
// once as a completed subshell command (the wait-side fire); the
// exit inside the region contributes no fire of its own.
// GNU: exactly 1, and -e stops the script.
#[test]
fn issue2482_t133_11_subshell_exit_statement_fires_once() {
    let dir = test_dir("11");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         ( exit 1 )\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 1);
    assert!(!stdout(&output).contains("after"));
    let _ = fs::remove_dir_all(&dir);
}

// 12 — a pipeline whose last stage is an exit-terminated function
// inside the EXIT action: the stage is a subshell, so the pipeline
// completes with status 1 and is adjudicated once at the pipeline
// level; -e then aborts the action. GNU: exactly 1, no
// continuation line.
#[test]
fn issue2482_t133_12_pipeline_stage_exit_in_action_fires_once() {
    let dir = test_dir("12");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         g() { exit 1; }\n\
         trap \"true | g; echo act-after\" EXIT\n\
         true\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 1);
    assert!(!stdout(&output).contains("act-after"));
    let _ = fs::remove_dir_all(&dir);
}

// 13 — the bats cell-133 shape distilled: the ERR action is itself
// a function (bats_error_trap) printing the failure callback; the
// suite body fails once, and the EXIT teardown chain ends in an
// inner `exit`. GNU prints the callback exactly once.
#[test]
fn issue2482_t133_13_bats_mini_callback_once() {
    let dir = test_dir("13");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         bats_error_trap() { echo \"failure callback\"; }\n\
         trap bats_error_trap ERR\n\
         teardown_inner() { exit 1; }\n\
         teardown() { teardown_inner; }\n\
         trap teardown EXIT\n\
         setup_suite() { false; }\n\
         setup_suite\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output).matches("failure callback").count(), 1);
    let _ = fs::remove_dir_all(&dir);
}

// 14 — boundary guard: an ordinary failure inside an EXIT-action
// function still adjudicates (fires once) and -e aborts the
// action. GNU: exactly 1, no continuation.
#[test]
fn issue2482_t133_14_action_failure_still_fires_and_aborts() {
    let dir = test_dir("14");
    let output = run_script(
        &dir,
        "case.sh",
        "set -eET\n\
         trap \"echo ERRFIRE\" ERR\n\
         g() { false; echo g-after; }\n\
         trap \"g\" EXIT\n\
         true\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 1);
    assert!(!stdout(&output).contains("g-after"));
    let _ = fs::remove_dir_all(&dir);
}

// 15 — boundary guard: without -e, an exit-terminated chain in
// the EXIT action stays silent (the errexit conversion site is
// not the only thing keeping it quiet — no site may fire).
// GNU: zero fires.
#[test]
fn issue2482_t133_15_no_errexit_chain_stays_silent() {
    let dir = test_dir("15");
    let output = run_script(
        &dir,
        "case.sh",
        "set -ET\n\
         trap \"echo ERRFIRE\" ERR\n\
         g() { exit 1; }\n\
         trap \"g\" EXIT\n\
         true\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(errfire_count(&output), 0);
    let _ = fs::remove_dir_all(&dir);
}

// 16 — boundary guard: without -e, nested ordinary failures keep
// their per-frame adjudication (the failing command fires, and
// the function call returning failure fires again at its caller).
// GNU: exactly 2, continuations print.
#[test]
fn issue2482_t133_16_no_errexit_failure_chain_fires_per_frame() {
    let dir = test_dir("16");
    let output = run_script(
        &dir,
        "case.sh",
        "set -ET\n\
         trap \"echo ERRFIRE\" ERR\n\
         g() { false; }\n\
         h() { g; echo h-after; }\n\
         h\n\
         echo after\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "ERRFIRE\nERRFIRE\nh-after\nafter\n");
    let _ = fs::remove_dir_all(&dir);
}
