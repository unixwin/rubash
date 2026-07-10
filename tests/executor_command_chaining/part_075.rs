use super::super::*;
use std::fs;

#[test]
fn test_while_command_redirect_creates_file_without_iterations() {
    let output_path = "target/rubash-while-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("while false; do echo bad; done > {output_path}");
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
fn test_while_command_redirects_body_stdout() {
    let output_path = "target/rubash-while-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "while true; do echo loop; break; done > {output_path}; echo done >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "loop\ndone\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_until_command_redirect_creates_file_without_iterations() {
    let output_path = "target/rubash-until-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("until true; do echo bad; done > {output_path}");
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
fn test_while_command_here_string_feeds_condition_read() {
    let output_path = "target/rubash-while-command-herestring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("while read value; do echo got:$value; done <<< alpha > {output_path}");
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
fn test_until_command_here_string_feeds_body_read() {
    let output_path = "target/rubash-until-command-herestring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "until false; do read value; echo got:$value; break; done <<< alpha > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "got:alpha\n");
    let _ = fs::remove_file(output_path);
}
