use super::super::*;
use std::fs;

#[test]
fn test_ulimit_appends_stderr() {
    let error_path = "target/rubash-ulimit-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("ulimit -g 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("ulimit: -g: invalid option"));
    assert!(error.contains("ulimit: usage:"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_alias_redirects_output() {
    let output_path = "target/rubash-alias-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("alias ll='ls -l'; alias -p > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alias ll='ls -l'\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_appends_output() {
    let output_path = "target/rubash-alias-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("alias ll='ls -l'; alias -p >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\nalias ll='ls -l'\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_alias_updates_alias_table() {
    let output_path = "target/rubash-builtin-alias-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin alias ll='ls -l'; builtin alias -p > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alias ll='ls -l'\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_unalias_updates_alias_table() {
    let output_path = "target/rubash-builtin-unalias-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("builtin alias gone='echo bad'; builtin unalias gone; alias -p > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_rejects_invalid_option() {
    let error_path = "target/rubash-alias-invalid-option-error.txt";
    let status_path = "target/rubash-alias-invalid-option-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("alias -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("alias: -x: invalid option"));
    assert!(error.contains("alias: usage: alias [-p] [name[=value] ... ]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_alias_accepts_double_dash_before_operand() {
    let output_path = "target/rubash-alias-double-dash-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("alias -- a='echo ok'; alias a > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alias a='echo ok'\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unalias_a_clears_all_and_ignores_operands() {
    let output_path = "target/rubash-unalias-a-output.txt";
    let status_path = "target/rubash-unalias-a-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input =
        format!("alias a=1 b=2; unalias -a a; echo $? > {status_path}; alias -p > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_unalias_rejects_invalid_option() {
    let error_path = "target/rubash-unalias-invalid-option-error.txt";
    let status_path = "target/rubash-unalias-invalid-option-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("unalias -x a 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("unalias: -x: invalid option"));
    assert!(error.contains("unalias: usage: unalias [-a] name [name ...]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_unalias_accepts_double_dash_before_dash_name() {
    let status_path = "target/rubash-unalias-double-dash-status.txt";
    let _ = fs::remove_file(status_path);
    let input = format!("alias -- -a=1; unalias -- -a; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_alias_redirects_stderr() {
    let error_path = "target/rubash-alias-stderr-output.txt";
    let status_path = "target/rubash-alias-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("alias no_such_alias 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("alias: no_such_alias: not found"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_alias_appends_stderr() {
    let error_path = "target/rubash-alias-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("alias no_such_alias 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("alias: no_such_alias: not found"));
    let _ = fs::remove_file(error_path);
}
