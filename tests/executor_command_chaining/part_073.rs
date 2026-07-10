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
