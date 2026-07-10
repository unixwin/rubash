use super::super::*;
use std::fs;

#[test]
fn test_times_redirects_output() {
    let output_path = "target/rubash-times-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("times > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_times_appends_output() {
    let output_path = "target/rubash-times-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("times >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\n0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_times_ignores_non_option_arguments() {
    let output_path = "target/rubash-times-extra-args-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("times ignored > {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_times_redirects_stderr() {
    let error_path = "target/rubash-times-stderr-output.txt";
    let status_path = "target/rubash-times-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("times -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("rubash: times: -x: invalid option"));
    assert!(error.contains("times: usage: times"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_times_appends_stderr() {
    let error_path = "target/rubash-times-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("times -x 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("rubash: times: -x: invalid option"));
    assert!(error.contains("times: usage: times"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_help_redirects_output() {
    let output_path = "target/rubash-help-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("help -s help > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "help: help [-dms] [pattern ...]\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_help_appends_output() {
    let output_path = "target/rubash-help-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("help -s help >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\nhelp: help [-dms] [pattern ...]\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_help_redirects_stderr() {
    let error_path = "target/rubash-help-stderr-output.txt";
    let status_path = "target/rubash-help-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("help -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("help: -x: invalid option"));
    assert!(error.contains("help: usage: help [-dms] [pattern ...]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_help_appends_stderr() {
    let error_path = "target/rubash-help-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("help -x 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("help: -x: invalid option"));
    assert!(error.contains("help: usage: help [-dms] [pattern ...]"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_dirs_redirects_output() {
    let output_path = "target/rubash-dirs-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("PWD=/tmp/rubash-dirs dirs > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "/tmp/rubash-dirs\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_dirs_appends_output() {
    let output_path = "target/rubash-dirs-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("PWD=/tmp/rubash-dirs dirs >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\n/tmp/rubash-dirs\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_dirs_redirects_output() {
    let output_path = "target/rubash-builtin-dirs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("PWD=/tmp/rubash-builtin-dirs builtin dirs > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "/tmp/rubash-builtin-dirs\n"
    );
    let _ = fs::remove_file(output_path);
}
