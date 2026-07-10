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

#[test]
fn test_for_command_input_redirect_feeds_body_reads() {
    let input_path = "target/rubash-for-command-input.txt";
    let output_path = "target/rubash-for-command-input-output.txt";
    fs::write(input_path, "alpha\nbeta\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for item in first second; do read value; echo $item:$value; done < {input_path} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    assert!(ast.commands[0].redirect_in.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "first:alpha\nsecond:beta\n"
    );
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_here_string_feeds_body_read() {
    let output_path = "target/rubash-for-command-herestring-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("for item in one; do read value; echo $item:$value; done <<< alpha > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].for_command.is_some());
    assert!(ast.commands[0].here_string.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "one:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_keeps_brace_group_body_command() {
    let output_path = "target/rubash-for-brace-group-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for item in one; do {{ echo $item; }} done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let body = &ast.commands[0].for_command.as_ref().unwrap().body;
    assert!(body.iter().any(|command| command.brace_group.is_some()));
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "one\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_for_command_input_redirect_feeds_body_read() {
    let input_path = "target/rubash-arithmetic-for-command-input.txt";
    let output_path = "target/rubash-arithmetic-for-command-input-output.txt";
    fs::write(input_path, "alpha\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for (( i = 0; i < 1; i++ )); do read value; echo $i:$value; done < {input_path} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0]
        .for_command
        .as_ref()
        .and_then(|for_command| for_command.arithmetic.as_ref())
        .is_some());
    assert!(ast.commands[0].redirect_in.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0:alpha\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}
