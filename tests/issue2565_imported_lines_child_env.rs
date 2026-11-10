//! rubash#2565: an imported, exported `LINES` must reach the child
//! environment exactly like `COLUMNS` does.
//!
//! GNU anchor: variables.c initialize_shell_variables binds every
//! imported environment entry with att_exported|att_imported
//! (variables.c:513-516), and bind_variable's exported rebuild
//! (variables.c:3214-3215) treats LINES and COLUMNS identically —
//! both names appear in a child's environment with their imported
//! values. rubash let the imported COLUMNS through but dropped
//! LINES somewhere on the import -> export -> compose chain.
//!
//! Expectations verified against GNU bash 5.3.20:
//! `env LINES=999 COLUMNS=200 bash -c 'env'` lists both names.

use std::process::Command;

fn run_with_env(script: &str) -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LINES", "999")
        .env("COLUMNS", "200")
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn imported_lines_and_columns_reach_child_env() {
    // The `env` builtin with no arguments prints the composed child
    // environment — the same pairs a spawned child would receive.
    let (stdout, stderr) = run_with_env("env");
    assert!(
        stdout.lines().any(|line| line == "LINES=999"),
        "child env must carry the imported LINES=999; stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.lines().any(|line| line == "COLUMNS=200"),
        "child env must carry the imported COLUMNS=200; stdout: {stdout} stderr: {stderr}"
    );
}

#[test]
fn imported_lines_and_columns_stay_exported() {
    // GNU: `declare -x NAME` is attribute mode and prints nothing;
    // the printed form comes from `declare -p NAME`.
    let (stdout, _stderr) = run_with_env("declare -p LINES COLUMNS");
    assert!(
        stdout.contains("declare -x LINES=\"999\""),
        "LINES must stay an exported 999; stdout: {stdout}"
    );
    assert!(
        stdout.contains("declare -x COLUMNS=\"200\""),
        "COLUMNS must stay an exported 200; stdout: {stdout}"
    );
}

#[test]
fn imported_lines_and_columns_reach_spawned_child() {
    // A real spawned child (`/usr/bin/env` as an external process)
    // receives the composed environment — the issue's acceptance shape.
    let (stdout, stderr) = run_with_env("/usr/bin/env");
    assert!(
        stdout.lines().any(|line| line == "LINES=999"),
        "spawned child env must carry LINES=999; stdout: {stdout} stderr: {stderr}"
    );
    assert!(
        stdout.lines().any(|line| line == "COLUMNS=200"),
        "spawned child env must carry COLUMNS=200; stdout: {stdout} stderr: {stderr}"
    );
}

#[test]
fn imported_lines_and_columns_expand_to_imported_values() {
    let (stdout, _stderr) = run_with_env("echo \"L=$LINES C=$COLUMNS\"");
    assert_eq!(stdout, "L=999 C=200\n");
}
