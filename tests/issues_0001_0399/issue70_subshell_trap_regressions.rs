//! Issue unixwin/niubash#70 secondary regression: a subshell body that
//! registers an EXIT trap must run it when the subshell exits, while the
//! subshell state (variables) is still live, and the parent must not see the
//! trap. GNU anchor: execute_cmd.c subshell exit -> shell.c exit_shell ->
//! trap.c run_exit_trap.

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

#[test]
fn subshell_exit_trap_fires_after_body() {
    let (stdout, stderr, code) = rubash(r#"( trap "echo T" EXIT; echo body )"#);
    assert_eq!(stdout, "body\nT\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn subshell_exit_trap_fires_without_other_body_output() {
    let (stdout, _, _) = rubash(r#"( trap "echo T" EXIT )"#);
    assert_eq!(stdout, "T\n");
}

#[test]
fn subshell_exit_trap_sees_subshell_variables() {
    let (stdout, _, _) = rubash(r#"( v=9; trap "echo T=$v" EXIT )"#);
    assert_eq!(stdout, "T=9\n");
}

#[test]
fn parent_exit_trap_does_not_fire_inside_subshell() {
    let (stdout, _, code) = rubash(r#"trap "echo TOP" EXIT; ( echo inner ); echo after; exit 0"#);
    assert_eq!(stdout, "inner\nafter\nTOP\n");
    assert_eq!(code, Some(0));
}

#[test]
fn subshell_exit_trap_action_exit_replaces_status() {
    let (stdout, _, code) = rubash(r#"( trap "echo T; exit 5" EXIT; echo body ); echo "rc=$?""#);
    assert_eq!(stdout, "body\nT\nrc=5\n");
    assert_eq!(code, Some(0));
}

#[test]
fn subshell_status_preserved_by_default_trap() {
    let (_, _, code) = rubash(r#"( trap "echo T" EXIT; exit 7 )"#);
    assert_eq!(code, Some(7));
}

#[test]
fn command_substitution_exit_trap_unchanged() {
    let (stdout, _, _) = rubash(r#"out="$(trap "echo TC" EXIT; echo body)"; echo "[$out]""#);
    assert_eq!(stdout, "[body\nTC]\n");
}
