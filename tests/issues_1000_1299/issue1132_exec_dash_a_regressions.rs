//! rubash#1132: `exec -a name command ...` must pass `name` as the
//! command's argv[0].
//!
//! GNU anchor: `builtins/exec.def`: `-a` supplies the alternate
//! argv[0] to execve (the login-shell `-name` convention included).
//! rubash parsed the flag but still passed the resolved program
//! path, so the child's `$0` showed the path instead of the alias.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::Command;

fn rubash_with_self(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("SELF", env!("CARGO_BIN_EXE_rubash"))
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The reported case: the re-exec'd shell sees `$0` = the alias.
#[test]
fn exec_dash_a_sets_child_argv_zero() {
    let (stdout, _, code) = rubash_with_self("exec -a customargv \"$SELF\" -c 'echo zero=$0'");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "zero=customargv\n");
}

/// The dash-prefixed login form passes through verbatim.
#[test]
fn exec_dash_a_login_style_name() {
    let (stdout, _, code) = rubash_with_self("exec -a -bash \"$SELF\" -c 'echo zero=$0'");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "zero=-bash\n");
}

/// Control: without `-a`, argv[0] is the program path as invoked.
#[test]
fn exec_without_dash_a_keeps_invoked_path() {
    let (stdout, _, code) = rubash_with_self("exec \"$SELF\" -c 'echo zero=$0'");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, format!("zero={}\n", env!("CARGO_BIN_EXE_rubash")));
}
