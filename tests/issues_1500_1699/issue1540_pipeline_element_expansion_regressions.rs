//! rubash#1540 — a pipeline element whose word expansion fails must
//! die alone: no output, the diagnostic exactly once, the class's
//! pre-set status in PIPESTATUS, and no trace left in the parent.
//!
//! GNU forks every element (execute_cmd.c execute_pipeline ->
//! execute_simple_command) and expands its words once inside the
//! child; an expand_word_error (subst.c) DISCARDs there — the
//! element produces nothing and exits with the status the class set
//! before the jump (1 for arithmetic / bad substitution / failglob,
//! EX_BADUSAGE 2 for a failed `${var:=word}` bind, the trampoline's
//! 1 for a no_args DISCARD raised by the element builtin itself).
//! The parent only observes the element's exit status.
//!
//! Expectations measured against GNU bash 5.3.20 with script files
//! in this environment (byte-level stdout, diagnostic counts,
//! PIPESTATUS; each form run 3x with identical results).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-1540-{}-{tag}-{n}", std::process::id()));
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
fn issue_1540_arithmetic_error_in_first_element() {
    let (stdout, stderr, code) =
        run_script("echo $((1/0)) | cat | wc -l\necho \"ps=${PIPESTATUS[*]} rc=$?\"\n");
    assert_eq!(stdout, "0\nps=1 0 0 rc=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("division by 0").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1540_arithmetic_error_in_middle_and_last_elements() {
    let (stdout, stderr, code) =
        run_script("echo hi | echo $((1/0)) | cat\necho \"rc=$? ps=${PIPESTATUS[*]}\"\n");
    assert_eq!(stdout, "rc=0 ps=0 1 0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("division by 0").count(),
        1,
        "stderr: {stderr}"
    );
    let (stdout, stderr, code) =
        run_script("printf '' | echo $((1/0))\necho \"ps=${PIPESTATUS[*]}\"\n");
    assert_eq!(stdout, "ps=0 1\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("division by 0").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1540_arithmetic_error_in_external_element() {
    let (stdout, stderr, code) = run_script("cat $((1/0)) | wc -l\necho \"ps=${PIPESTATUS[*]}\"\n");
    assert_eq!(stdout, "0\nps=1 0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("division by 0").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1540_arithmetic_error_inside_comsub_inside_element_prints_once() {
    // The substitution child reports the error; the outer echo then
    // runs with the empty result, so the element itself succeeds —
    // but the diagnostic must appear exactly once, not once per
    // expansion pass.
    let (stdout, stderr, code) =
        run_script("echo $(echo $((1/0))) | cat\necho \"ps=${PIPESTATUS[*]}\"\n");
    assert_eq!(stdout, "\nps=0 0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("division by 0").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1540_bad_substitution_in_element() {
    let (stdout, stderr, code) =
        run_script("echo ${#:} | cat\necho \"ps=${PIPESTATUS[*]} rc=$?\"\necho after\n");
    assert_eq!(stdout, "ps=1 0 rc=0\nafter\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("bad substitution").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1540_failglob_in_element() {
    let (stdout, stderr, code) = run_script(
        "shopt -s failglob\necho nope* | cat\necho \"ps=${PIPESTATUS[*]}\"\necho after\n",
    );
    assert_eq!(stdout, "ps=1 0\nafter\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("no match: nope*").count(),
        1,
        "stderr: {stderr}"
    );
    // Same through an external-routed element's argv builder.
    let (stdout, stderr, code) =
        run_script("shopt -s failglob\ncat nope* | wc -l\necho \"ps=${PIPESTATUS[*]}\"\n");
    assert_eq!(stdout, "0\nps=1 0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("no match: nope*").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1540_param_assign_failure_in_element_does_not_leak() {
    // The element dies with EX_BADUSAGE 2; the parent list is
    // untouched (previously the latched failure killed the script).
    let (stdout, stderr, code) =
        run_script("readonly v\necho ${v:=x} | cat\necho \"ps=${PIPESTATUS[*]}\"\n");
    assert_eq!(stdout, "ps=2 0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("v: readonly variable").count(),
        1,
        "stderr: {stderr}"
    );
    let (stdout, stderr, code) =
        run_script("readonly v\ncat ${v:=x} | wc -l\necho \"ps=${PIPESTATUS[*]}\"\necho after\n");
    assert_eq!(stdout, "0\nps=2 0\nafter\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("v: readonly variable").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1540_builtin_too_many_args_in_element_folds_to_1() {
    // The execute_subshell_builtin_or_function trampoline converts
    // the element builtin's own no_args DISCARD to subshell_exit(1).
    let (stdout, stderr, code) = run_script("exit 1 2 | cat\necho \"ps=${PIPESTATUS[*]}\"\n");
    assert_eq!(stdout, "ps=1 0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("exit: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
    // Same for the loop-control builtins as direct elements; the
    // enclosing loop in the parent is unaffected.
    let (stdout, stderr, code) = run_script(
        "for i in 1 2; do break 1 2 | cat; echo \"iter$i ps=${PIPESTATUS[*]}\"; done\necho done\n",
    );
    assert_eq!(
        stdout, "iter1 ps=1 0\niter2 ps=1 0\ndone\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("break: too many arguments").count(),
        2,
        "stderr: {stderr}"
    );
    let (stdout, stderr, code) = run_script(
        "f() { return 1 2 | cat; echo \"in-f ps=${PIPESTATUS[*]}\"; }\nf\necho \"outer=$?\"\n",
    );
    assert_eq!(stdout, "in-f ps=1 0\nouter=0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("return: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
}

#[test]
fn issue_1540_function_element_too_many_args_keeps_2() {
    // The trampoline fold applies only when the builtin IS the
    // element. Inside a function element's body the DISCARD keeps
    // the pre-set EX_BADUSAGE (GNU 5.3.20 probe), and the body list
    // is abandoned at the failing command.
    let (stdout, stderr, code) =
        run_script("f() { exit 1 2; echo IN; }\nf | cat\necho \"ps=${PIPESTATUS[*]}\"\n");
    assert_eq!(stdout, "ps=2 0\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr.matches("exit: too many arguments").count(),
        1,
        "stderr: {stderr}"
    );
}
