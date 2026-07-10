use super::super::*;
use std::fs;

#[test]
fn test_case_fallthrough_executes_next_clause_body() {
    let output_path = "target/rubash-case-fallthrough-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "case beta in alpha) echo alpha > {output_path} ;; beta) echo beta > {output_path} ;& gamma) echo gamma >> {output_path} ;; *) echo star > {output_path} ;; esac"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].case_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "beta\ngamma\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_test_next_terminator_matches_later_clause() {
    let output_path = "target/rubash-case-test-next-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "case beta in alpha) echo alpha > {output_path} ;; beta) echo beta > {output_path} ;;& b*) echo bstar >> {output_path} ;; *) echo star > {output_path} ;; esac"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].case_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "beta\nbstar\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_keyword_definition_executes_body() {
    let output_path = "target/rubash-function-keyword-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function greet {{ echo hi > {output_path}; }}; greet");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].function_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hi\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parenthesized_function_definition_executes_body() {
    let output_path = "target/rubash-parenthesized-function-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("greet() ( echo hi > {output_path} ); greet");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].function_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hi\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parenthesized_function_body_runs_in_subshell() {
    let output_path = "target/rubash-parenthesized-function-subshell-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=outer; change() ( value=inner; echo $value > {output_path} ); \
         change; echo $value >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[1].function_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "inner\nouter\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_posix_return_keeps_prefix_assignment() {
    let output_path = "target/rubash-posix-return-prefix-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -o posix; value=outer; change() {{ value=inner return 5; }}; \
         change; printf '%s:%s\\n' \"$?\" \"$value\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "5:inner\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_posix_function_prefix_assignment_restores_when_unchanged() {
    let output_path = "target/rubash-posix-function-prefix-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -o posix; value=outer; noop() {{ return 5; }}; \
         value=temp noop; printf '%s:%s\\n' \"$?\" \"$value\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "5:outer\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_without_in_iterates_positional_params() {
    let output_path = "target/rubash-for-default-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -- alpha beta; for item; do echo $item >> {output_path}; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[1].for_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_explicit_empty_in_does_not_iterate() {
    let output_path = "target/rubash-for-empty-in-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -- alpha beta; for item in; do echo $item > {output_path}; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[1].for_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_for_loop_exit_status_matches_body_or_zero_iterations() {
    let output_path = "target/rubash-for-status-output.txt";
    let _ = fs::remove_file(output_path);
    let loop_only_tokens = tokenize("for item in one; do false; done");
    let loop_only_ast = parse(&loop_only_tokens);
    let mut loop_only_executor = Executor::new();
    let loop_only_result = loop_only_executor.execute_ast(&loop_only_ast);
    assert!(loop_only_result.is_ok());
    assert_eq!(loop_only_executor.last_exit_code(), 1);

    let input = format!(
        "for item in one; do false; done; echo $? > {output_path}; false; for item in; do true; done; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_for_loop_executes() {
    let output_path = "target/rubash-arithmetic-for-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for (( i = 0; i < 3; i++ )); do echo $i >> {output_path}; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0]
        .for_command
        .as_ref()
        .and_then(|for_command| for_command.arithmetic.as_ref())
        .is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_expands_positional_count() {
    let output_path = "target/rubash-arithmetic-positional-count-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "function f {{ echo arith:$(( $# )) > {output_path}; for ((i=1; i<=$#; i++)); do echo loop:$i >> {output_path}; done; }}; f a b"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "arith:2\nloop:1\nloop:2\n"
    );
    let _ = fs::remove_file(output_path);
}
