//! rubash#1013: `eval` with an empty/blank/comment-only string returns 0
//! instead of preserving `$?` (GNU builtins/evalstring.c:351
//! parse_and_execute inits last_result to EXECUTION_SUCCESS).

use std::process::Command;

fn run(script: &str) -> (String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn eval_empty_string_resets_status_to_zero() {
    let (out, code) = run("false; eval \"\"; echo \"rc=$?\"");
    assert_eq!(out, "rc=0\n");
    assert_eq!(code, 0);
}

#[test]
fn eval_blank_and_comment_only_strings_reset_status_to_zero() {
    let (out, _) = run("false; eval \"   \"; echo \"rc=$?\"");
    assert_eq!(out, "rc=0\n");
    let (out, _) = run("false; eval \"# just a comment\"; echo \"rc=$?\"");
    assert_eq!(out, "rc=0\n");
}

#[test]
fn eval_bare_and_nonempty_unchanged() {
    let (out, _) = run("false; eval; echo \"rc=$?\"");
    assert_eq!(out, "rc=0\n");
    let (out, _) = run("false; eval \"echo hi\"; echo \"rc=$?\"");
    assert_eq!(out, "hi\nrc=0\n");
}
