use super::super::*;
use std::fs;

#[test]
fn test_alias_introduced_arithmetic_for_executes_loop() {
    let output_path = "target/rubash-alias-arithmetic-for-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias f=for; \
         f (( i = 0; i < 3; i++ )); do echo $i; done > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_arithmetic_for_accepts_brace_group_body() {
    let output_path = "target/rubash-alias-arithmetic-for-brace-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias f=for; \
         f (( i = 0; i < 2; i++ )); {{ echo $i; }} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_arithmetic_for_accepts_empty_init() {
    let output_path = "target/rubash-alias-arithmetic-for-empty-init-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias f=for; (( i = 0 )); \
         f (( ; i < 3; i++ )); do echo $i; done > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_arithmetic_for_accepts_empty_update() {
    let output_path = "target/rubash-alias-arithmetic-for-empty-update-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias f=for; \
         f (( i = 0; i < 3; )); do echo $i; (( i++ )); done > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n");
    let _ = fs::remove_file(output_path);
}
