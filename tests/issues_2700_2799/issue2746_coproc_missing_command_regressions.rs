//! rubash#2746 (B6-R8): a bare `coproc` (or one followed by a
//! pipeline/list operator) fell out of the coproc parser into the
//! simple-command path — `coproc: command not found' (127) for the
//! bare form, a silent success for `coproc | cat'. GNU's grammar is
//! `COPROC [WORD] shell_command` (parse.y:1125-1175): with no
//! shell_command the following token is a syntax error (rc 2). A
//! redirection-only simple command IS a shell_command and stays
//! valid.
use std::{env, process::Command};

fn run(source: &str) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(source)
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn assert_syntax_error_near(source: &str, token: &str) {
    let (stdout, stderr, code) = run(source);
    assert_eq!(
        code, 2,
        "source: {source}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(&format!("syntax error near unexpected token `{token}'")),
        "source: {source}\nexpected blame `{token}', stderr: {stderr}"
    );
}

#[test]
fn issue2746_bare_coproc_is_a_syntax_error() {
    assert_syntax_error_near("coproc", "newline");
}

#[test]
fn issue2746_coproc_before_a_pipeline_is_a_syntax_error() {
    assert_syntax_error_near("coproc | cat", "|");
}

#[test]
fn issue2746_coproc_before_a_separator_is_a_syntax_error() {
    assert_syntax_error_near("coproc; echo after", ";");
}

#[test]
fn issue2746_coproc_before_and_list_is_a_syntax_error() {
    assert_syntax_error_near("coproc && echo after", "&&");
}

#[test]
fn issue2746_redirection_only_coproc_is_valid() {
    let dir = env::temp_dir().join(format!("rubash-2746-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let file = dir.join("out.txt");
    let source = format!("coproc > {}\necho rc-marker", file.display());
    let (stdout, stderr, code) = run(&source);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "rc-marker\n");
    assert!(file.exists(), "redirection was not applied");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2746_simple_and_named_coprocs_still_work() {
    let (stdout, stderr, code) = run("coproc echo hi\necho done");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "done\n");

    let (stdout, stderr, code) = run("coproc CP { echo hi; }\necho done");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "done\n");
}
