//! Issue rubash#3994: a process-substitution redirection on a
//! command-less assignment statement was diagnosed as a missing
//! file (`x=1 < <(sleep 1)` reported `<(sleep 1): No such file or
//! directory`, rc=1).
//!
//! GNU model: execute_null_command (execute_cmd.c:4626-4635) runs
//! do_redirections over the same redirect list as any simple
//! command, and the redirect words' process substitutions were
//! materialized during word expansion (redirection_expand,
//! redir.c:298). A null command therefore opens the substitution
//! like any other command does — silently, rc=0 — while a QUOTED
//! `"<(cmd)"` spelling stays a literal filename and a genuinely
//! missing file still fails. Expected outputs verified against
//! GNU bash 5.3.20 (script files).

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

fn rubash_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3994-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn issue3994_assignment_input_procsub_is_silent() {
    let (stdout, stderr, code) = rubash("x=1 < <(sleep 1)\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn issue3994_redirect_only_input_procsub_is_silent() {
    let (stdout, stderr, code) = rubash("< <(sleep 1)\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn issue3994_group_residual_face_is_silent() {
    // The #3964 residue shape: correct stdout, bogus stderr.
    let (stdout, stderr, code) =
        rubash("echo hi | { x=$(</dev/stdin) < <(sleep 1); echo \"[$x]\"; }");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[hi]\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn issue3994_input_procsub_body_runs() {
    let dir = scratch("body");
    let (stdout, stderr, code) = rubash_in(&dir, "x=1 < <(touch marker)\nsleep 0.5\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert!(dir.join("marker").exists(), "procsub body never ran");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3994_output_procsub_sink_file_lands() {
    let dir = scratch("sink");
    let (stdout, stderr, code) = rubash_in(&dir, "x=1 > >(cat > out)\nsleep 0.5\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert!(dir.join("out").exists(), "output procsub never spawned");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue3994_numbered_fd_input_procsub_is_silent() {
    let (stdout, stderr, code) = rubash("x=1 9< <(echo hi)\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn issue3994_failure_order_procsub_first_still_reports_bad_file() {
    let (stdout, stderr, code) = rubash("x=1 < <(sleep 1) < /nonexistent3994\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("/nonexistent3994: No such file or directory"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("sleep"), "stderr: {stderr}");
}

#[test]
fn issue3994_failure_order_bad_file_first_reports_bad_file() {
    let (stdout, stderr, code) = rubash("x=1 < /nonexistent3994 < <(sleep 1)\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("/nonexistent3994: No such file or directory"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("sleep"), "stderr: {stderr}");
}

#[test]
fn issue3994_function_body_null_command_is_silent() {
    let (stdout, stderr, code) =
        rubash("f() { x=1 < <(sleep 1); echo inrc=$?; }\nf\necho outrc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "inrc=0\noutrc=0\n");
    assert_eq!(stderr, "", "stderr: {stderr}");
}

#[test]
fn issue3994_quoted_procsub_spelling_stays_literal() {
    // Control: the quoted spelling is a literal filename in GNU too.
    let (stdout, stderr, code) = rubash("x=1 < \"<(sleep 1)\"\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("<(sleep 1): No such file or directory"),
        "stderr: {stderr}"
    );
}

#[test]
fn issue3994_plain_missing_file_still_fails() {
    // Control: a genuinely missing file keeps its diagnostic.
    let (stdout, stderr, code) = rubash("x=1 < /nonexistent3994\necho rc=$?");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("/nonexistent3994: No such file or directory"),
        "stderr: {stderr}"
    );
}
