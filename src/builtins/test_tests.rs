use super::*;
use std::collections::HashMap;
use std::fs;

fn run(args: &[&str], bracket: bool) -> (i32, String) {
    let env_vars = HashMap::new();
    run_with_env(args, bracket, &env_vars)
}

fn run_with_env(args: &[&str], bracket: bool, env_vars: &HashMap<String, String>) -> (i32, String) {
    let mut stderr = Vec::new();
    let status = execute_with_stderr(args.iter().copied(), bracket, env_vars, &mut stderr).unwrap();
    (status, String::from_utf8(stderr).unwrap())
}

#[test]
fn empty_expression_is_false() {
    assert_eq!(run(&[], false).0, EXECUTION_FAILURE);
}

#[test]
fn single_non_empty_string_is_true() {
    assert_eq!(run(&["hello"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&[""], false).0, EXECUTION_FAILURE);
}

#[test]
fn supports_string_and_numeric_binary_operators() {
    assert_eq!(run(&["a", "=", "a"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["2", "-lt", "3"], false).0, EXECUTION_SUCCESS);
}

#[test]
fn supports_not_and_logical_operators() {
    assert_eq!(run(&["!", ""], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["x", "-a", ""], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["x", "-o", ""], false).0, EXECUTION_SUCCESS);
}

#[test]
fn supports_shell_option_unary_operator() {
    let mut env_vars = HashMap::new();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    crate::builtins::set::set_with_io(["-o", "errexit"], &mut env_vars, &mut stdout, &mut stderr)
        .unwrap();

    assert_eq!(
        run_with_env(&["-o", "errexit"], false, &env_vars).0,
        EXECUTION_SUCCESS
    );
    crate::builtins::set::set_with_io(["+o", "errexit"], &mut env_vars, &mut stdout, &mut stderr)
        .unwrap();
    assert_eq!(
        run_with_env(&["-o", "errexit"], false, &env_vars).0,
        EXECUTION_FAILURE
    );
    assert_eq!(
        run_with_env(&["-o", "no_such_option"], false, &env_vars).0,
        EXECUTION_FAILURE
    );
}

#[test]
fn leading_file_operator_is_unary_not_logical_and() {
    assert_eq!(run(&["-a", "Cargo.toml"], false).0, EXECUTION_SUCCESS);
}

#[test]
fn modified_since_read_unary_operator_checks_existing_file() {
    let path = "target/rubash-test-n-unary.txt";
    let missing = "target/rubash-test-n-unary-missing.txt";
    let _ = fs::create_dir_all("target");
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(missing);
    fs::write(path, "data").unwrap();

    assert_eq!(run(&["-N", path], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-N", missing], false).0, EXECUTION_FAILURE);

    let _ = fs::remove_file(path);
}

#[test]
fn ownership_unary_operators_check_existing_file() {
    let path = "target/rubash-test-owner-unary.txt";
    let missing = "target/rubash-test-owner-unary-missing.txt";
    let _ = fs::create_dir_all("target");
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(missing);
    fs::write(path, "data").unwrap();

    let expected = if cfg!(unix) {
        EXECUTION_SUCCESS
    } else {
        EXECUTION_FAILURE
    };
    assert_eq!(run(&["-O", path], false).0, expected);
    assert_eq!(run(&["-G", path], false).0, expected);
    assert_eq!(run(&["-O", missing], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["-G", missing], false).0, EXECUTION_FAILURE);

    let _ = fs::remove_file(path);
}

#[test]
fn supports_parenthesized_logical_expressions() {
    assert_eq!(
        run(&["(", "", "-o", "x", ")", "-a", ""], false).0,
        EXECUTION_FAILURE
    );
    assert_eq!(
        run(&["\\(", "", "-o", "x", "\\)", "-a", "x"], false).0,
        EXECUTION_SUCCESS
    );
}

#[test]
fn bracket_requires_closing_bracket() {
    let (status, stderr) = run(&["x"], true);

    assert_eq!(status, EX_BADUSAGE);
    assert!(stderr.contains("missing `]'"));
}
