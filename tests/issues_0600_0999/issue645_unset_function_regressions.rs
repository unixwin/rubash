#![cfg(windows)]

use std::process::{Command, Stdio};

fn run(script: &str) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash -c");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn unset_variable_keeps_same_named_function() {
    let (code, stdout, stderr) = run(r#"foo() { echo FN; }; foo=VAR; unset foo; foo"#);
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "FN\n");
    assert!(stderr.is_empty(), "stderr={stderr:?}");
}

#[test]
fn unset_without_variable_falls_back_to_function() {
    let (code, stdout, stderr) = run("foo() { echo FN; }; unset foo; foo");
    assert_eq!(code, Some(127), "stdout={stdout:?} stderr={stderr:?}");
    assert!(stdout.is_empty(), "stdout={stdout:?}");
}
