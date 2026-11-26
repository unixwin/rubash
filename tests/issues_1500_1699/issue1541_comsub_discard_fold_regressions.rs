//! rubash#1541 — DISCARD-class abandonments inside a command
//! substitution body fold EX_BADUSAGE 2 to EXECUTION_FAILURE 1.
//!
//! GNU runs a comsub body through parse_and_execute inside the forked
//! child (subst.c command_substitute), whose DISCARD catch
//! (builtins/evalstring.c) forces
//! `last_result = last_command_exit_value = EXECUTION_FAILURE` and, in
//! a subshell environment, stops the body outright. The script-level
//! reader_loop catch (eval.c) instead KEEPS the pre-set status, and a
//! `( )` region at script level keeps it too (execute_in_subshell in
//! execute_cmd.c). So the no_args class (break/continue/exit/return
//! with too many arguments — builtins/common.c no_args:
//! set_exit_status(EX_BADUSAGE) + jump_to_top_level(DISCARD)) reads 2
//! at script level but 1 anywhere inside a `$()` tree, while other
//! classes pass through untouched (a plain status, the EXITPROG-class
//! `break x`, ...).
//!
//! Expectations measured against GNU bash 5.3.20 with script files in
//! this environment (byte-level stdout, diagnostic counts, statuses;
//! pipeline-free forms run 3x with identical results).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-1541-{}-{tag}-{n}", std::process::id()));
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
fn issue_1541_exit_too_many_args_in_comsub_folds_to_1() {
    let (stdout, stderr, code) = run_script("x=$(exit 1 2)\necho \"s=$?\"\n");
    assert_eq!(stdout, "s=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("exit: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1541_comsub_body_stops_at_the_discard() {
    // The parse_and_execute catch stops the whole body in a subshell
    // environment: commands after the failing builtin never run.
    let (stdout, stderr, code) = run_script("x=$(exit 1 2; echo after-in)\necho \"x=[$x] s=$?\"\n");
    assert_eq!(stdout, "x=[] s=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let (stdout, stderr, code) =
        run_script("x=$(echo a; exit 1 2; echo b)\necho \"x=[$x] s=$?\"\n");
    assert_eq!(stdout, "x=[a] s=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1541_break_too_many_args_in_comsub_inside_outer_loop() {
    // The discard kills only the substitution child; the outer loop
    // (in the parent shell) iterates on.
    let (stdout, stderr, code) =
        run_script("for i in 1 2; do x=$(break 1 2); echo \"iter$i s=$?\"; done\necho done\n");
    assert_eq!(stdout, "iter1 s=1\niter2 s=1\ndone\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("break: too many arguments").count(),
        2,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1541_continue_too_many_args_in_comsub_loop() {
    let (stdout, stderr, code) =
        run_script("x=$(for i in 1 2; do continue 1 2; done)\necho \"s=$?\"\n");
    assert_eq!(stdout, "s=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("continue: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1541_return_too_many_args_in_comsub_inside_function() {
    let (stdout, stderr, code) =
        run_script("f() { x=$(return 1 2); echo \"in-f s=$?\"; }\nf\necho \"outer=$?\"\n");
    assert_eq!(stdout, "in-f s=1\nouter=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("return: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1541_eval_inside_comsub_folds_and_stops() {
    let (stdout, stderr, code) =
        run_script("x=$(eval 'exit 1 2\necho IN')\necho \"x=[$x] s=$?\"\n");
    assert_eq!(stdout, "x=[] s=1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("exit: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1541_subshell_region_inside_comsub_folds() {
    // A `( )` region closed by the discard inside a comsub body takes
    // the folded status; the body then continues after the region.
    let (stdout, stderr, code) =
        run_script("x=$( (exit 1 2); echo \"rs=$?\"; echo in )\necho \"s=$? x=[$x]\"\n");
    assert_eq!(stdout, "s=0 x=[rs=1\nin]\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1541_script_level_forms_keep_status_2() {
    // The reader_loop catch and execute_in_subshell keep the pre-set
    // EX_BADUSAGE — no fold outside a substitution body.
    let (stdout, stderr, code) = run_script("( exit 1 2 )\necho \"s=$?\"\n");
    assert_eq!(stdout, "s=2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let (stdout, stderr, code) =
        run_script("eval 'for i in 1 2; do break 1 2; done'\necho \"s=$?\"\n");
    assert_eq!(stdout, "s=2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("break: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1541_other_status_classes_pass_through_comsub() {
    // Plain statuses and the EXITPROG class (non-numeric argument)
    // are not DISCARDs of the no_args shape and must not fold.
    let (stdout, _, code) = run_script("x=$(exit 300)\necho \"s=$?\"\n");
    assert_eq!(stdout, "s=44\n");
    assert_eq!(code, Some(0));
    let (stdout, _, code) = run_script("x=$(false)\necho \"s=$?\"\n");
    assert_eq!(stdout, "s=1\n");
    assert_eq!(code, Some(0));
    let (stdout, stderr, code) = run_script("x=$(for i in 1 2; do break x; done)\necho \"s=$?\"\n");
    assert_eq!(stdout, "s=2\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("numeric argument required").count(),
        1,
        "stderr: {stderr}"
    );
}
