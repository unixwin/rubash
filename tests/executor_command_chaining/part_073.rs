use super::super::*;
use std::fs;

#[test]
fn test_if_true_executes_then_body() {
    let output_path = "target/rubash-if-true-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("if true; then echo yes > {output_path}; fi");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_elif_true_executes_after_false_if() {
    let output_path = "target/rubash-elif-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("if false; then echo no > {output_path}; elif true; then echo yes > {output_path}; else echo bad > {output_path}; fi");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_condition_command_runs_before_then() {
    let condition_path = "target/rubash-if-condition-side-effect.txt";
    let output_path = "target/rubash-if-command-output.txt";
    let _ = fs::remove_file(condition_path);
    let _ = fs::remove_file(output_path);
    let input = format!(
        "if printf cond > {condition_path}; then echo yes > {output_path}; else echo no > {output_path}; fi"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(condition_path).unwrap(), "cond");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(condition_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_condition_command_status_selects_else() {
    let output_path = "target/rubash-if-command-false-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("if false; then echo yes > {output_path}; else echo no > {output_path}; fi");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "no\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_flattened_arithmetic_comparison_selects_else() {
    let output_path = "target/rubash-if-arith-false-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("if 0 == 1; then echo yes > {output_path}; else echo no > {output_path}; fi");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "no\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_command_redirects_then_body_stdout() {
    let output_path = "target/rubash-if-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("if true; then echo yes; fi > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].if_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_command_redirect_creates_file_without_matching_branch() {
    let output_path = "target/rubash-if-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("if false; then echo bad; fi > {output_path}");
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
fn test_if_command_redirects_condition_stdout() {
    let output_path = "target/rubash-if-command-redirect-condition-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("if printf cond; then echo yes; fi > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "condyes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_command_redirects_body_stderr() {
    let error_path = "target/rubash-if-command-redirect-error.txt";
    let _ = fs::remove_file(error_path);
    let input = format!("if true; then echo err >&2; fi 2> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(error_path).unwrap(), "err\n");
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_if_command_input_redirect_feeds_condition_and_body() {
    let input_path = "target/rubash-if-command-input.txt";
    let output_path = "target/rubash-if-command-input-output.txt";
    fs::write(input_path, "alpha\nbeta\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!(
        "if read first; then read second; echo $first/$second; fi < {input_path} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha/beta\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_command_here_string_feeds_condition() {
    let output_path = "target/rubash-if-command-herestring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("if read value; then echo got:$value; fi <<< alpha > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "got:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_if_executes_then_branch() {
    let output_path = "target/rubash-alias-if-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias i=if\ni true; then echo yes > {output_path}; else echo no > {output_path}; fi"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_if_executes_else_branch() {
    let output_path = "target/rubash-alias-if-else-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases\nalias i=if\ni false; then echo yes > {output_path}; else echo no > {output_path}; fi"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "no\n");
    let _ = fs::remove_file(output_path);
}
