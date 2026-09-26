//! Governance S6 / doc 3.6: a `( )` subshell is GNU execute_cmd.c:1576
//! execute_in_subshell — a forked child whose whole mutable shell state is a
//! copy of the parent's. Alias and function definitions/mutations inside the
//! subshell must not reach the parent (previously they leaked: the flat
//! subshell save/restore list covered ~8 fields and never included
//! aliases/functions). Verified byte-for-byte against WSL GNU Bash 5.3.0.

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
fn alias_defined_in_subshell_does_not_leak() {
    let (stdout, stderr, _) = rubash(
        "alias ll='echo PARENT'\n( alias ll='echo CHILD'; alias newa='echo NEW' )\nalias ll\nalias newa",
    );
    assert_eq!(stdout, "alias ll='echo PARENT'\n");
    assert!(
        stderr.contains("alias: newa: not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn function_defined_in_subshell_does_not_leak() {
    let (stdout, stderr, _) = rubash(
        "f() { echo PARENT_F; }\n( f() { echo CHILD_F; }; g() { echo NEW_G; } )\ndeclare -F f\ndeclare -F g",
    );
    assert_eq!(stdout, "f\n");
    assert_eq!(stderr, "");
}

#[test]
fn subshell_alias_override_restores_parent_alias() {
    let (stdout, _, _) = rubash("alias ll='echo PARENT'\n( alias ll='echo CHILD' )\nalias ll");
    assert_eq!(stdout, "alias ll='echo PARENT'\n");
}

#[test]
fn nested_subshell_definitions_do_not_leak() {
    let (stdout, stderr, _) =
        rubash("( ( alias d='echo DEEP'; df() { echo DF; } ) )\nalias d\ndeclare -F df");
    assert_eq!(stdout, "");
    assert!(stderr.contains("alias: d: not found"), "stderr: {stderr}");
}

#[test]
fn function_body_paren_subshell_does_not_leak() {
    let (stdout, stderr, _) =
        rubash("h() ( alias hb='echo HB'; hbf() { echo HBF; } )\nh\nalias hb\ndeclare -F hbf");
    assert_eq!(stdout, "");
    assert!(stderr.contains("alias: hb: not found"), "stderr: {stderr}");
}

#[test]
fn command_substitution_definitions_do_not_leak() {
    let (stdout, _, _) =
        rubash("x=$( alias cx='echo CS'; cf() { echo CF; }; echo done )\necho $x\ndeclare -F cf");
    assert_eq!(stdout, "done\n");
}

#[test]
fn subshell_function_mutation_restores_parent_body() {
    let (stdout, _, _) = rubash("f() { echo PARENT_F; }\n( f() { echo CHILD_F; } )\nf");
    assert_eq!(stdout, "PARENT_F\n");
}

#[test]
fn subshell_set_positional_and_vars_still_isolated() {
    // Regression guard: the wholesale state restore must keep the
    // historically-isolated fields isolated too.
    let (stdout, _, _) = rubash("v=P\n( v=C; set -- a b c )\necho $v\necho $#");
    assert_eq!(stdout, "P\n0\n");
}
