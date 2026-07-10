use super::super::*;
use std::fs;

#[test]
fn test_exit_help_redirects_output_and_exits_usage() {
    let output_path = "target/rubash-exit-help-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("exit --help > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(2))));
    assert_eq!(executor.last_exit_code(), 2);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("exit: exit [n]\n"));
    assert!(output.contains("Exit the shell."));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exit_help_appends_output_and_exits_usage() {
    let output_path = "target/rubash-exit-help-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("exit --help >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(2))));
    assert_eq!(executor.last_exit_code(), 2);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("before\nexit: exit [n]\n"));
    assert!(output.contains("Exit the shell."));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exit_invalid_number_redirects_stderr() {
    let status_path = "target/rubash-exit-invalid-number-status.txt";
    let error_path = "target/rubash-exit-invalid-number-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!("exit abc 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("exit: abc: numeric argument required"));
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_exit_too_many_arguments_redirects_stderr() {
    let status_path = "target/rubash-exit-too-many-status.txt";
    let error_path = "target/rubash-exit-too-many-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!("exit 1 2 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("exit: too many arguments"));
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_eval_redirects_entire_output() {
    let output_path = "target/rubash-eval-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "old\n").unwrap();
    let input = format!("eval 'echo alpha; echo beta' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_eval_appends_entire_output() {
    let output_path = "target/rubash-eval-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("eval 'echo alpha; echo beta' >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\nalpha\nbeta\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_eval_expands_assignment_lhs_to_array_name() {
    let output_path = "target/rubash-eval-array-lhs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() {{ local -a r; r=(three two one); eval $1=\\( \\\"\\$\\{{r\\[@\\]\\}}\\\" \\); }}; \
         f arr; declare -p arr > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "declare -a arr=([0]=\"three\" [1]=\"two\" [2]=\"one\")\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_eval_invalid_option_redirects_stderr() {
    let status_path = "target/rubash-eval-invalid-option-status.txt";
    let error_path = "target/rubash-eval-invalid-option-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!("eval -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("eval: -Z: invalid option"));
    assert!(error.contains("eval: usage: eval"));
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_eval_body_redirects_stderr() {
    let status_path = "target/rubash-eval-body-error-status.txt";
    let error_path = "target/rubash-eval-body-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!("eval 'no_such_eval_cmd' 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "127\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("no_such_eval_cmd: command not found"));
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_builtin_eval_redirects_entire_output() {
    let output_path = "target/rubash-builtin-eval-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "old\n").unwrap();
    let input = format!("builtin eval 'echo alpha; echo beta' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_complete_empty_state_redirects_no_output() {
    let output_path = "target/rubash-complete-output.txt";
    let status_path = "target/rubash-complete-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("complete > {output_path}; echo $? > {status_path}");
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
fn test_complete_invalid_option_reports_usage() {
    let error_path = "target/rubash-complete-error.txt";
    let status_path = "target/rubash-complete-error-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("complete -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("complete: -x: invalid option\n"));
    assert!(error.contains("complete: usage: complete "));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}
