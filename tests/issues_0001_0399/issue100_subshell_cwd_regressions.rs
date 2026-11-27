//! Issue unixwin/niubash#100 regression: `( list )` subshells must isolate
//! the process working directory. GNU runs the body via execute_in_subshell
//! -> make_child (execute_cmd.c), so the forked child owns its own cwd and a
//! `cd` inside never reaches the parent. Rubash runs the body in place, so
//! the cwd is part of the saved subshell environment alongside env_vars,
//! positional parameters, and the typed variable store.

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
fn subshell_cd_does_not_leak_to_parent() {
    let (stdout, _, code) = rubash("pwd; ( cd .. ); pwd");
    let mut lines = stdout.lines();
    let before = lines.next().expect("first pwd");
    let after = lines.next().expect("second pwd");
    assert_eq!(before, after);
    assert_eq!(lines.next(), None);
    assert_eq!(code, Some(0));
}

#[test]
fn function_subshell_body_cd_does_not_leak() {
    let (stdout, _, code) = rubash("pwd; f() ( cd ..; pwd ); f; pwd");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], lines[2]);
    assert_eq!(code, Some(0));
}

#[test]
fn inner_subshell_cd_restores_to_outer_subshell_cwd() {
    // ( cd A; ( cd B; pwd ); pwd ): the inner subshell restores A, the
    // outer restores the script cwd.
    let (stdout, _, code) = rubash("pwd; ( cd /; ( cd ..; pwd ); pwd ); pwd");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], lines[3]);
    assert_eq!(code, Some(0));
}

#[test]
fn subshell_exit_restores_parent_cwd() {
    let (stdout, _, code) = rubash("pwd; ( cd ..; exit 3 ); echo rc=$?; pwd");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], lines[2]);
    assert_eq!(lines[1], "rc=3");
    assert_eq!(code, Some(0));
}

#[test]
fn brace_group_cd_still_leaks() {
    // `{ list; }` is NOT a subshell: GNU keeps its cd in the parent.
    let (stdout, _, code) = rubash("pwd; { cd ..; }; pwd");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_ne!(lines[0], lines[1]);
    assert_eq!(code, Some(0));
}

#[test]
fn function_subshell_body_keeps_assignments_local() {
    // `f() ( v=inner )` runs the body in a subshell: the typed variable
    // store must be saved and restored like env_vars (the flat
    // subshell/subshell_end path, parser function_command.rs).
    let (stdout, _, code) = rubash(r#"v=outer; f() ( v=inner; echo "in:$v" ); f; echo "out:$v""#);
    assert_eq!(stdout, "in:inner\nout:outer\n");
    assert_eq!(code, Some(0));
}

#[test]
fn function_subshell_body_keeps_positional_params_local() {
    let (stdout, _, code) =
        rubash(r#"set -- a b; f() ( set -- x y; echo "in:$1 $2" ); f; echo "out:$1 $2""#);
    assert_eq!(stdout, "in:x y\nout:a b\n");
    assert_eq!(code, Some(0));
}
