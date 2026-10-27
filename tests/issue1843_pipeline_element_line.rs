//! rubash#1843: a pipeline element inside a function body reported the
//! function's DEFINITION line for its diagnostics instead of the
//! element's own line. GNU execute_cmd.c:936 (execute_simple_command)
//! stamps SET_LINE_NUMBER (command->value.Simple->line) for EVERY
//! simple element — execute_pipeline forks each element into that same
//! funnel — so the element's own line owns its diagnostics wherever the
//! pipeline runs.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (script-file
//! probes, 2026-10-27).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1843_function_body_pipeline_element_reports_its_own_line() {
    // The pipeline sits on line 3 of the definition; the command-not-found
    // diagnostic must cite line 3, not the definition's opening line 1.
    let (_, stderr, _) = rubash("f() {\n  true\n  nosuchcmd_xyz_1843 | cat\n}\nf\n");
    assert!(
        stderr.contains("line 3: nosuchcmd_xyz_1843: command not found"),
        "element diagnostic must carry the element line, got: {stderr:?}"
    );
    assert!(
        !stderr.contains("line 1:"),
        "definition line leaked into the element diagnostic: {stderr:?}"
    );
}

#[test]
fn issue1843_nested_function_pipeline_element_reports_its_own_line() {
    // g's body (line 2) holds the pipeline; f merely calls g.
    let (_, stderr, _) = rubash("g() {\n  nosuchcmd_xyz_1843 | cat\n}\nf() {\n  g\n}\nf\n");
    assert!(
        stderr.contains("line 2: nosuchcmd_xyz_1843: command not found"),
        "nested element diagnostic must carry the element line, got: {stderr:?}"
    );
}

#[test]
fn issue1843_top_level_pipeline_element_line_unchanged() {
    let (_, stderr, _) = rubash("true\nnosuchcmd_xyz_1843 | cat\n");
    assert!(
        stderr.contains("line 2: nosuchcmd_xyz_1843: command not found"),
        "top-level element line regressed: {stderr:?}"
    );
}

#[test]
fn issue1843_function_body_simple_command_line_unchanged() {
    // The simple (non-pipeline) element already reported its own line;
    // the pipeline stamp must not disturb it.
    let (_, stderr, _) = rubash("f() {\n  true\n  nosuchcmd_xyz_1843\n}\nf\n");
    assert!(
        stderr.contains("line 3: nosuchcmd_xyz_1843: command not found"),
        "simple element line regressed: {stderr:?}"
    );
}

#[test]
fn issue1843_function_body_pipeline_redirect_failure_reports_element_line() {
    // SET_LINE_NUMBER precedes do_redirections in execute_simple_command,
    // so the element's redirect failure also carries the element line.
    let (_, stderr, code) = rubash(
        "f() {\n  true\n  echo hi 2>/nonexist_1843/e | cat\n  echo \"st=$? ps=[${PIPESTATUS[*]}]\"\n}\nf\n",
    );
    assert!(
        stderr.contains("line 3: /nonexist_1843/e: No such file or directory"),
        "redirect diagnostic must carry the element line, got: {stderr:?}"
    );
    assert_eq!(code, Some(0));
}
