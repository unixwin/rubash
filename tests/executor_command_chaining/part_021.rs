use super::super::*;
use std::fs;

#[test]
fn test_export_appends_stderr() {
    let error_path = "target/rubash-export-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("export -Z 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("rubash: export: -Z: invalid option"));
    assert!(error.contains("export: usage:"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_readonly_redirects_stderr() {
    let error_path = "target/rubash-readonly-stderr-output.txt";
    let status_path = "target/rubash-readonly-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("readonly -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("readonly: -Z: invalid option"));
    assert!(error.contains("readonly: usage:"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_readonly_appends_stderr() {
    let error_path = "target/rubash-readonly-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("readonly -Z 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("readonly: -Z: invalid option"));
    assert!(error.contains("readonly: usage:"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_export_p_appends_output() {
    let output_path = "target/rubash-export-p-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("export RUBASH_EXPORT_APPEND=value; export -p >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("before\n"));
    assert!(output.contains("declare -x RUBASH_EXPORT_APPEND=\"value\"\n"));
    std::env::remove_var("RUBASH_EXPORT_APPEND");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pwd_redirects_output() {
    let output_path = "target/rubash-pwd-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("PWD=/tmp/rubash-pwd-test pwd > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "/tmp/rubash-pwd-test\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pwd_appends_output() {
    let output_path = "target/rubash-pwd-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("PWD=/tmp/rubash-pwd-test pwd >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\n/tmp/rubash-pwd-test\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pwd_redirects_stderr() {
    let error_path = "target/rubash-pwd-stderr-output.txt";
    let status_path = "target/rubash-pwd-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);

    let input = format!("pwd -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("rubash: pwd: -x: invalid option"));
    assert!(error.contains("pwd: usage: pwd [-LP]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_pwd_appends_stderr() {
    let error_path = "target/rubash-pwd-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();

    let input = format!("pwd -x 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("rubash: pwd: -x: invalid option"));
    assert!(error.contains("pwd: usage: pwd [-LP]"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_hash_redirects_output() {
    let output_path = "target/rubash-hash-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("hash -p /tmp/rubash-cat cat; hash -t cat > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "/tmp/rubash-cat\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_hash_appends_output() {
    let output_path = "target/rubash-hash-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("hash -p /tmp/rubash-cat cat; hash -t cat >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\n/tmp/rubash-cat\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_hash_empty_table_reports_success() {
    let error_path = "target/rubash-hash-empty-stderr-output.txt";
    let status_path = "target/rubash-hash-empty-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("hash 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("hash: hash table empty"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}
