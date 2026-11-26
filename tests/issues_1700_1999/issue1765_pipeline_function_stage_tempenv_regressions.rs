//! rubash#1765: a pipeline FUNCTION stage dropped its prefix (temporary)
//! assignments entirely — `a=0; f() { echo $a; }; a=9 f | cat` printed the
//! outer `0` where GNU bash 5.3.20 prints `9`.
//!
//! GNU model: a pipeline element's forked child runs the element's simple
//! command itself, so the assignment words are processed in the child —
//! subst.c:13153 expand_word_list_internal picks assign_in_env
//! (variables.c:3536) to build the temporary_env, and execute_function
//! (execute_cmd.c:5181) receives it as the function's variable context via
//! push_context (execute_cmd.c:5233/5252). Compound prefix words therefore
//! bind as the exported scalar WINDOW text (literal list text; for `+=`,
//! the old cell's scalar view prepended), never as a live array — the
//! `declare -x a="(x y)"` shapes below are GNU's own output, measured on
//! the 5.3.20 oracle from script files.
//!
//! Adjacent face pinned as measured (NOT as sometimes described): prefix
//! assignments before a NON-pipeline function call are visible inside the
//! body and the caller's bindings are restored after the call returns, in
//! every shape probed on the oracle (existing / unset / append / array /
//! exported). The fix touches only the pipeline stage's state build, so
//! these faces must not move.

use std::process::Command;

fn shell_bin() -> std::path::PathBuf {
    // NOTE: the `bash` bin is a thin forwarder to the INSTALLED Niubash
    // (src/bin/bash.rs) — testing through it would exercise the old engine.
    // The real engine binary is `rubash`.
    if let Some(path) = option_env!("CARGO_BIN_EXE_rubash") {
        return std::path::PathBuf::from(path);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(if cfg!(windows) {
            "rubash.exe"
        } else {
            "rubash"
        })
}

fn run(script: &str) -> (i32, String, String) {
    run_with_flags(&[], script)
}

fn run_posix(script: &str) -> (i32, String, String) {
    run_with_flags(&["--posix"], script)
}

fn run_with_flags(flags: &[&str], script: &str) -> (i32, String, String) {
    let output = Command::new(shell_bin())
        .args(flags)
        .args(["-c", script])
        .output()
        .expect("run shell");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn issue1765_pipeline_stage_scalar_prefix_is_visible_in_body() {
    let (rc, stdout, _) = run("a=0; f() { echo \"scalar:$a\"; }; a=9 f | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "scalar:9\n");
}

#[test]
fn issue1765_pipeline_stage_append_prefix_uses_old_value() {
    let (rc, stdout, _) = run("a=0; f() { echo \"append:$a\"; }; a+=9 f | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "append:09\n");
}

#[test]
fn issue1765_pipeline_stage_indexed_literal_binds_window_text() {
    // GNU: the compound word binds as an exported scalar holding the
    // literal list text — element reads see that text, count is 1.
    let (rc, stdout, _) =
        run("f() { echo \"elem0:${a[0]} n=${#a[@]}\"; declare -p a; }; a=(x y) f | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "elem0:(x y) n=1\ndeclare -x a=\"(x y)\"\n");
}

#[test]
fn issue1765_pipeline_stage_indexed_append_prepends_scalar_view() {
    let (rc, stdout, _) = run("a=(p); f() { declare -p a; }; a+=(q) f | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -x a=\"p(q)\"\n");
}

#[test]
fn issue1765_pipeline_stage_assoc_literal_binds_window_text() {
    let (rc, stdout, _) = run("declare -A h; f() { declare -p h; }; h=([k1]=v1 [k2]=v2) f | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -x h=\"([k1]=v1 [k2]=v2)\"\n");
}

#[test]
fn issue1765_pipeline_stage_assoc_append_binds_literal_only() {
    // An associative cell's scalar view is empty, so the append window is
    // exactly the literal list text.
    let (rc, stdout, _) =
        run("declare -A h=([k0]=base); f() { declare -p h; }; h+=([k1]=new) f | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "declare -x h=\"([k1]=new)\"\n");
}

#[test]
fn issue1765_pipeline_stage_prefix_applies_in_middle_stage() {
    let (rc, stdout, _) = run("a=0; f() { echo \"mid:$a\"; }; echo start | a=7 f | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "mid:7\n");
}

#[test]
fn issue1765_pipeline_stage_rhs_expands_against_old_value() {
    let (rc, stdout, _) = run("a=1; f() { echo \"rhs:$a\"; }; a=$((a+10)) f | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "rhs:11\n");
}

#[test]
fn issue1765_pipeline_stage_multiple_assignments_expand_left_to_right() {
    let (rc, stdout, _) = run("f() { echo \"multi:$a,$b\"; }; a=1 b=$a f | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "multi:1,1\n");
    let (rc, stdout, _) = run("a=0; f() { echo \"order:$a,$b\"; }; b=$a a=1 f | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "order:1,0\n");
}

#[test]
fn issue1765_pipeline_stage_window_does_not_leak_to_sibling_stage() {
    let (rc, stdout, _) =
        run("a=0; f() { echo \"s:$a\"; }; g() { echo \"g:$a\"; cat; }; a=9 f | g | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "g:0\ns:9\n");
}

#[test]
fn issue1765_pipeline_stage_body_writes_do_not_leak_to_outer_shell() {
    let (rc, stdout, _) =
        run("a=0; f() { a=changed; echo \"body:$a\"; }; a=9 f | cat; echo \"outer:$a\"");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "body:changed\nouter:0\n");
}

#[test]
fn issue1765_pipeline_stage_readonly_failure_still_runs_body_by_default() {
    // GNU default mode: the readonly diagnostic is printed and the
    // function still runs with the (unbindable) old value visible.
    let (rc, stdout, stderr) = run("f() { echo \"body:$a\"; }; readonly a=0; a=9 f | cat");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "body:0\n");
    assert!(
        stderr.contains("readonly variable"),
        "expected readonly diagnostic, got stderr: {stderr}"
    );
}

#[test]
fn issue1765_pipeline_stage_readonly_failure_skips_body_in_posix_mode() {
    let (rc, stdout, stderr) = run_posix(
        "f() { echo body-ran; }; readonly a=0; a=9 f | cat; echo \"st:${PIPESTATUS[0]}\"",
    );
    assert_eq!(rc, 0);
    assert_eq!(stdout, "st:1\n");
    assert!(
        stderr.contains("readonly variable"),
        "expected readonly diagnostic, got stderr: {stderr}"
    );
}

#[test]
fn issue1765_pipeline_stage_arith_expansion_failure_skips_body() {
    let (rc, stdout, stderr) =
        run("f() { echo body-ran; }; x=$((1/0)) f | cat; echo \"st:${PIPESTATUS[0]}\"");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "st:1\n");
    assert!(
        !stderr.is_empty(),
        "expected an arithmetic diagnostic on stderr"
    );
}

#[test]
fn issue1765_nonpipeline_prefix_visible_inside_and_restored_after() {
    let (rc, stdout, _) = run("a=0; f() { echo \"in:$a\"; }; a=9 f; echo \"after:$a\"");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "in:9\nafter:0\n");
}

#[test]
fn issue1765_nonpipeline_prefix_on_unset_var_restored_after() {
    let (rc, stdout, _) = run("unset b; f() { :; }; b=5 f; echo \"after:${b-unset}\"");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "after:unset\n");
}

#[test]
fn issue1765_nonpipeline_append_and_exported_faces_restored_after() {
    let (rc, stdout, _) = run("a=0; f() { :; }; a+=9 f; echo \"append-after:$a\"");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "append-after:0\n");
    let (rc, stdout, _) = run("export a=0; f() { :; }; a=9 f; echo \"exp-after:$a\"");
    assert_eq!(rc, 0);
    assert_eq!(stdout, "exp-after:0\n");
}
