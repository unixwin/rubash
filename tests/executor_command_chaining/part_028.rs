use super::super::*;
use std::fs;

#[test]
fn test_unalias_redirects_stderr() {
    let error_path = "target/rubash-unalias-stderr-output.txt";
    let status_path = "target/rubash-unalias-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("unalias no_such_alias 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("unalias: no_such_alias: not found"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_unalias_appends_stderr() {
    let error_path = "target/rubash-unalias-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("unalias no_such_alias 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("unalias: no_such_alias: not found"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_set_redirects_output() {
    let output_path = "target/rubash-set-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("RUBASH_SET_REDIRECT=value set > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("RUBASH_SET_REDIRECT=value\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_set_appends_output() {
    let output_path = "target/rubash-set-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("RUBASH_SET_APPEND=value set >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("before\n"));
    assert!(output.contains("RUBASH_SET_APPEND=value\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_set_redirects_stderr() {
    let error_path = "target/rubash-set-stderr-output.txt";
    let status_path = "target/rubash-set-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("set -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("rubash: set: -Z: invalid option"));
    assert!(error.contains("set: usage:"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_set_appends_stderr() {
    let error_path = "target/rubash-set-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("set -Z 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("rubash: set: -Z: invalid option"));
    assert!(error.contains("set: usage:"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_unset_redirects_stderr() {
    let error_path = "target/rubash-unset-stderr-output.txt";
    let status_path = "target/rubash-unset-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("unset 1BAD 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("rubash: unset: `1BAD`: not a valid identifier"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_unset_appends_stderr() {
    let error_path = "target/rubash-unset-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("unset 1BAD 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("rubash: unset: `1BAD`: not a valid identifier"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_builtin_unset_removes_variable() {
    let output_path = "target/rubash-builtin-unset-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("RUBASH_BUILTIN_UNSET=value; builtin unset RUBASH_BUILTIN_UNSET; echo ${{RUBASH_BUILTIN_UNSET:-missing}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "missing\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unset_clears_export_attribute() {
    let output_path = target_test_path("rubash-unset-export-output.txt");
    let error_path = target_test_path("rubash-unset-export-error.txt");
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let input = format!(
        "export RUBASH_UNSET_EXPORT=value; unset RUBASH_UNSET_EXPORT; \
         printf '<%s>\\n' \"${{RUBASH_UNSET_EXPORT-unset}}\" > {shell_output_path}; \
         export -p >> {shell_output_path}; \
         declare -p RUBASH_UNSET_EXPORT 2> {shell_error_path}; \
         echo $? >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.starts_with("<unset>\n"));
    assert!(output.ends_with("1\n"));
    assert!(!output.contains("RUBASH_UNSET_EXPORT"));
    let error = fs::read_to_string(&error_path).unwrap();
    assert!(error.contains("declare: RUBASH_UNSET_EXPORT: not found"));
    std::env::remove_var("RUBASH_UNSET_EXPORT");
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
}

#[test]
fn test_unset_rejects_unset_readonly_variable() {
    let output_path = target_test_path("rubash-unset-readonly-output.txt");
    let error_path = target_test_path("rubash-unset-readonly-error.txt");
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let input = format!(
        "unset RUBASH_UNSET_READONLY; readonly RUBASH_UNSET_READONLY; \
         unset RUBASH_UNSET_READONLY 2> {shell_error_path}; echo $? > {shell_output_path}; \
         declare -p RUBASH_UNSET_READONLY >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "1\ndeclare -r RUBASH_UNSET_READONLY\n"
    );
    let error = fs::read_to_string(&error_path).unwrap();
    assert!(error.contains("unset: RUBASH_UNSET_READONLY: cannot unset: readonly variable"));
    std::env::remove_var("RUBASH_UNSET_READONLY");
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
}
