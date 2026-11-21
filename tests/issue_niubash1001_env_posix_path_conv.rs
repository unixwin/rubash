//! Regression tests for unixwin/niubash#1001: POSIX paths in environment
//! variable values are converted to Windows form when spawning native Windows
//! executables, while honoring `MSYS2_ENV_CONV_EXCL`, keeping URLs/non-path
//! values untouched, and preserving `PWD` verbatim.

#![cfg(windows)]

use std::process::Command;

fn run_rubash(script: &str, extra_env: &[(&str, &str)]) -> (Option<i32>, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.args(["-c", script]);
    for (name, value) in extra_env {
        command.env(name, value);
    }
    let output = command.output().expect("spawn rubash");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn prefix_and_exported_posix_paths_convert_for_native_exe() {
    let (code, stdout, stderr) = run_rubash(
        r#"FOO=/d/x cmd /c "echo FOO=%FOO%"; export CARGO_TARGET_DIR=/d/Temp/niucheck; cmd /c "echo CARGO=%CARGO_TARGET_DIR%""#,
        &[],
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().map(|l| l.trim()).collect();
    assert_eq!(lines, vec![r"FOO=D:\x", r"CARGO=D:\Temp\niucheck"]);
}

#[test]
fn env_builtin_converts_posix_paths_for_native_exe() {
    let (code, stdout, stderr) = run_rubash(
        r#"env CARGO_TARGET_DIR=/d/Temp/niucheck cmd /c "echo %CARGO_TARGET_DIR%""#,
        &[],
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout.trim(), r"D:\Temp\niucheck");
}

#[test]
fn colon_separated_posix_path_lists_convert_for_native_exe() {
    let (code, stdout, stderr) = run_rubash(
        r#"PYTHONPATH=/d/a:/c/Users/b cmd /c "echo %PYTHONPATH%""#,
        &[],
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout.trim(), r"D:\a;C:\Users\b");
}

#[test]
fn urls_and_non_path_values_stay_untouched() {
    let (code, stdout, stderr) = run_rubash(
        r#"URL1=http://example.com/d/x URL2=https://localhost:8080/a/b REL=foo/bar ROOT=/c cmd /c "echo %URL1% %URL2% %REL% %ROOT%""#,
        &[],
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        "http://example.com/d/x https://localhost:8080/a/b foo/bar /c"
    );
}

#[test]
fn msys2_env_conv_excl_excludes_matching_variables() {
    let (code, stdout, stderr) = run_rubash(
        r#"MSYS2_ENV_CONV_EXCL='FOO;CARGO_' FOO=/d/x CARGO_TARGET_DIR=/d/Temp/niucheck OTHER=/d/y cmd /c "echo %FOO% %CARGO_TARGET_DIR% %OTHER%""#,
        &[],
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout.trim(), r"/d/x /d/Temp/niucheck D:\y");

    let (code, stdout, stderr) = run_rubash(
        r#"MSYS2_ENV_CONV_EXCL='*' FOO=/d/x cmd /c "echo %FOO%""#,
        &[],
    );
    assert_eq!(code, Some(0), "stdout: {stdout} stderr: {stderr}");
    assert_eq!(stdout.trim(), "/d/x");
}
