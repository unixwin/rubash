use super::super::*;
use std::fs;

#[test]
fn test_exec_invalid_option_redirects_stderr() {
    let output_path = "target/rubash-exec-invalid-option-output.txt";
    let error_path = "target/rubash-exec-invalid-option-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("exec -Z 2> {error_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("exec: -Z: invalid option"));
    assert!(error.contains("exec: usage:"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_exec_a_requires_argument() {
    let output_path = "target/rubash-exec-a-requires-argument-output.txt";
    let error_path = "target/rubash-exec-a-requires-argument-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("exec -a 2> {error_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("exec: -a: option requires an argument"));
    assert!(error.contains("exec: usage:"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_builtin_exec_redirects_output() {
    let output_path = "target/rubash-builtin-exec-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin exec -a custom sh -c 'echo $0' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "custom\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_command_redirects_output() {
    let output_path = "target/rubash-builtin-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin command echo hello > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hello\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_echo_redirects_output() {
    let output_path = "target/rubash-command-echo-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("command echo hello > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hello\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_echo_appends_output() {
    let output_path = "target/rubash-command-echo-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("command echo hello >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "before\nhello\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_type_invokes_type_builtin() {
    let output_path = "target/rubash-command-type-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("function type {{ echo function-type; }}; command type -t echo > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "builtin\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_test_invokes_test_builtin() {
    let output_path = "target/rubash-command-test-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("command test 3 -eq 4; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_builtin_invokes_builtin_builtin() {
    let output_path = "target/rubash-command-builtin-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("command builtin echo hello > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hello\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_builtin_redirects_disabled_builtin_diagnostic() {
    let status_path = "target/rubash-command-builtin-disabled-status.txt";
    let error_path = "target/rubash-command-builtin-disabled-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input =
        format!("enable -n true; command builtin true 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("builtin: true: not a shell builtin"));
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_command_trap_redirects_output() {
    let output_path = "target/rubash-command-trap-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("trap 'echo bye' EXIT; command trap -p EXIT > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "trap -- 'echo bye' EXIT\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_kill_redirects_output() {
    let output_path = "target/rubash-command-kill-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("command kill -l HUP > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_source_missing_redirects_stderr() {
    let output_path = "target/rubash-command-source-missing-status.txt";
    let error_path = "target/rubash-command-source-missing-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input =
        format!("command source no_such_source_file 2> {error_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("no_such_source_file: No such file or directory"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}
