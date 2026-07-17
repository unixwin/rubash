use super::super::*;
use std::fs;

#[test]
fn test_bind_warns_without_line_editing_and_succeeds() {
    let error_path = "target/rubash-bind-error.txt";
    let status_path = "target/rubash-bind-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("bind 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("bind: warning: line editing not enabled"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_bind_invalid_option_returns_usage() {
    let error_path = "target/rubash-bind-invalid-error.txt";
    let status_path = "target/rubash-bind-invalid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("bind -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("bind: warning: line editing not enabled"));
    assert!(error.contains("bind: -Z: invalid option"));
    assert!(error.contains("bind: usage: bind [-lpsvPSVX]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_bind_option_requires_argument() {
    let error_path = "target/rubash-bind-missing-arg-error.txt";
    let status_path = "target/rubash-bind-missing-arg-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("bind -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("bind: -x: option requires an argument"));
    assert!(error.contains("bind: usage: bind [-lpsvPSVX]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_bind_compact_argument_option_consumes_rest_of_word() {
    let error_path = "target/rubash-bind-compact-arg-error.txt";
    let status_path = "target/rubash-bind-compact-arg-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("bind '-x\"\\C-x\":echo bound' 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("bind: warning: line editing not enabled"));
    assert!(!error.contains("invalid option"));
    assert!(!error.contains("option requires an argument"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_fc_without_history_returns_success() {
    let output_path = "target/rubash-fc-empty-output.txt";
    let status_path = "target/rubash-fc-empty-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("fc > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_fc_list_without_history_returns_success() {
    let output_path = "target/rubash-fc-list-output.txt";
    let status_path = "target/rubash-fc-list-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("fc -l > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_fc_invalid_option_returns_usage() {
    let error_path = "target/rubash-fc-invalid-error.txt";
    let status_path = "target/rubash-fc-invalid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("fc -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("fc: -x: invalid option"));
    assert!(error.contains("fc: usage: fc [-e ename] [-lnr]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_fc_edit_option_requires_argument() {
    let error_path = "target/rubash-fc-missing-edit-error.txt";
    let status_path = "target/rubash-fc-missing-edit-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("fc -e 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("fc: -e: option requires an argument"));
    assert!(error.contains("fc: usage: fc [-e ename] [-lnr]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_builtin_break_breaks_loop() {
    let output_path = "target/rubash-builtin-break-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("for value in a b; do builtin break; echo bad; done; echo done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "done\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_continue_continues_loop() {
    let output_path = "target/rubash-builtin-continue-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("for value in a b; do builtin continue; echo bad; done; echo done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "done\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_kill_redirects_output() {
    let output_path = "target/rubash-builtin-kill-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin kill -l HUP > {output_path}");
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
fn test_exec_redirects_output() {
    let output_path = "target/rubash-exec-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("exec -a custom sh -c 'echo $0' > {output_path}");
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
fn test_exec_combined_options_set_login_argv0() {
    let output_path = "target/rubash-exec-combined-options-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("exec -la custom sh -c 'echo $0' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "-custom\n");
    let _ = fs::remove_file(output_path);
}
