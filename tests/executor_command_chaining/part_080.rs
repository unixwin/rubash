use super::super::*;
use std::fs;

#[test]
fn test_combined_stdout_stderr_redirect_captures_brace_group() {
    let output_path = "target/rubash-combined-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("{{ echo out; no_such_combined_redirect_cmd; }} &> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 127);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("no_such_combined_redirect_cmd: command not found"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_combined_stdout_stderr_append_captures_brace_group() {
    let output_path = "target/rubash-combined-append-output.txt";
    fs::write(output_path, "first\n").unwrap();
    let input = format!("{{ echo out; no_such_combined_append_cmd; }} &>> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 127);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("first\n"));
    assert!(output.contains("out\n"));
    assert!(output.contains("no_such_combined_append_cmd: command not found"));
    let _ = fs::remove_file(output_path);
}
