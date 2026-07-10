use super::super::*;
use std::fs;

#[test]
fn test_for_command_redirects_body_stdout() {
    let output_path = "target/rubash-for-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for item in alpha beta; do echo $item; done > {output_path}; echo done >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alpha\nbeta\ndone\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_redirect_creates_file_without_iterations() {
    let output_path = "target/rubash-for-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for item in; do echo bad; done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_for_command_redirect_creates_file_without_iterations() {
    let output_path = "target/rubash-arithmetic-for-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for (( i = 0; i < 0; i++ )); do echo bad; done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0]
        .for_command
        .as_ref()
        .and_then(|for_command| for_command.arithmetic.as_ref())
        .is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}
