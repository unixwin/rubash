use super::super::*;
use std::fs;

#[test]
fn test_declare_plus_r_rejects_readonly_variable() {
    let output_path = target_test_path("rubash-declare-plus-r-output.txt");
    let error_path = target_test_path("rubash-declare-plus-r-error.txt");
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let input = format!(
        "declare -r RUBASH_DECLARE_PLUS_R=1; \
         declare +r RUBASH_DECLARE_PLUS_R 2> {shell_error_path}; echo $? > {shell_output_path}; \
         declare -p RUBASH_DECLARE_PLUS_R >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "1\ndeclare -r RUBASH_DECLARE_PLUS_R=\"1\"\n"
    );
    let error = fs::read_to_string(&error_path).unwrap();
    assert!(error.contains("declare: RUBASH_DECLARE_PLUS_R: readonly variable"));
    std::env::remove_var("RUBASH_DECLARE_PLUS_R");
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
}

#[test]
fn test_declare_assignment_rejects_readonly_variable() {
    let output_path = target_test_path("rubash-declare-readonly-assignment-output.txt");
    let error_path = target_test_path("rubash-declare-readonly-assignment-error.txt");
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let input = format!(
        "declare -r RUBASH_DECLARE_READONLY_ASSIGN=1; \
         declare RUBASH_DECLARE_READONLY_ASSIGN=2 2> {shell_error_path}; echo $? > {shell_output_path}; \
         declare -p RUBASH_DECLARE_READONLY_ASSIGN >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "1\ndeclare -r RUBASH_DECLARE_READONLY_ASSIGN=\"1\"\n"
    );
    let error = fs::read_to_string(&error_path).unwrap();
    assert!(error.contains("declare: RUBASH_DECLARE_READONLY_ASSIGN: readonly variable"));
    std::env::remove_var("RUBASH_DECLARE_READONLY_ASSIGN");
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
}

#[test]
fn test_declare_redirects_stderr() {
    let error_path = "target/rubash-declare-stderr-output.txt";
    let status_path = "target/rubash-declare-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("declare -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("declare: -Z: invalid option"));
    assert!(error.contains("declare: usage:"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_declare_appends_stderr() {
    let error_path = "target/rubash-declare-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("declare -Z 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("declare: -Z: invalid option"));
    assert!(error.contains("declare: usage:"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_typeset_invalid_option_uses_typeset_diagnostics() {
    let error_path = "target/rubash-typeset-invalid-option-error.txt";
    let status_path = "target/rubash-typeset-invalid-option-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("typeset -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("typeset: -Z: invalid option"));
    assert!(error.contains("typeset: usage:"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_declare_capital_g_is_accepted_as_noop_attribute() {
    let output_path = "target/rubash-declare-capital-g-output.txt";
    let error_path = "target/rubash-declare-capital-g-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "unset RUBASH_DECLARE_CAPITAL_G RUBASH_DECLARE_PLUS_CAPITAL_G; \
         declare -G RUBASH_DECLARE_CAPITAL_G 2> {error_path}; echo first:$? > {output_path}; \
         declare -p RUBASH_DECLARE_CAPITAL_G >> {output_path}; \
         declare +G RUBASH_DECLARE_PLUS_CAPITAL_G 2>> {error_path}; echo second:$? >> {output_path}; \
         declare -p RUBASH_DECLARE_PLUS_CAPITAL_G >> {output_path}; \
         unset RUBASH_DECLARE_CAPITAL_G; \
         declare -p RUBASH_DECLARE_CAPITAL_G 2>> {error_path}; echo missing:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "first:0\ndeclare -- RUBASH_DECLARE_CAPITAL_G\nsecond:0\ndeclare -- RUBASH_DECLARE_PLUS_CAPITAL_G\nmissing:1\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("declare: RUBASH_DECLARE_CAPITAL_G: not found"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_builtin_declare_assigns_variable() {
    let output_path = "target/rubash-builtin-declare-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin declare RUBASH_BUILTIN_DECLARE=value; echo $RUBASH_BUILTIN_DECLARE > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "value\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_typeset_assigns_variable() {
    let output_path = "target/rubash-builtin-typeset-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin typeset RUBASH_BUILTIN_TYPESET=value; echo $RUBASH_BUILTIN_TYPESET > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "value\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_export_p_redirects_output() {
    let output_path = "target/rubash-export-p-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("export RUBASH_EXPORT_REDIR=value; export -p > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(fs::read_to_string(output_path)
        .unwrap()
        .contains("declare -x RUBASH_EXPORT_REDIR=\"value\"\n"));
    std::env::remove_var("RUBASH_EXPORT_REDIR");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_export_p_omits_unexported_shell_variables() {
    let output_path = "target/rubash-export-p-unexported-output.txt";
    let _ = fs::remove_file(output_path);
    let old_value = std::env::var("RUBASH_EXPORT_INHERITED").ok();
    std::env::set_var("RUBASH_EXPORT_INHERITED", "from-env");
    let input = format!(
        "RUBASH_EXPORT_LOCAL=local; export RUBASH_EXPORT_MARKED=value; export -p > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    match old_value {
        Some(value) => std::env::set_var("RUBASH_EXPORT_INHERITED", value),
        None => std::env::remove_var("RUBASH_EXPORT_INHERITED"),
    }
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("declare -x RUBASH_EXPORT_INHERITED=\"from-env\"\n"));
    assert!(output.contains("declare -x RUBASH_EXPORT_MARKED=\"value\"\n"));
    assert!(!output.contains("RUBASH_EXPORT_LOCAL"));
    std::env::remove_var("RUBASH_EXPORT_MARKED");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_export_n_clears_export_attribute_without_unsetting_variable() {
    let output_path = "target/rubash-export-n-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "RUBASH_EXPORT_CLEAR=value; export RUBASH_EXPORT_CLEAR; export -n RUBASH_EXPORT_CLEAR; \
         printf '<%s>\\n' \"$RUBASH_EXPORT_CLEAR\" > {output_path}; \
         export -p >> {output_path}; printf '%s\\n' --- >> {output_path}; \
         declare -p RUBASH_EXPORT_CLEAR >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("<value>\n"));
    assert!(!output.contains("declare -x RUBASH_EXPORT_CLEAR="));
    assert!(output.contains("---\ndeclare -- RUBASH_EXPORT_CLEAR=\"value\"\n"));
    std::env::remove_var("RUBASH_EXPORT_CLEAR");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_export_without_assignment_marks_unset_variable() {
    let output_path = target_test_path("rubash-export-unset-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "unset RUBASH_EXPORT_UNSET; export RUBASH_EXPORT_UNSET; \
         printf '<%s>\\n' \"${{RUBASH_EXPORT_UNSET-unset}}\" > {shell_output_path}; \
         export -p >> {shell_output_path}; \
         printf '%s\\n' --- >> {shell_output_path}; \
         declare -p RUBASH_EXPORT_UNSET >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.starts_with("<unset>\n"));
    assert!(output.contains("declare -x RUBASH_EXPORT_UNSET\n"));
    assert!(output.ends_with("---\ndeclare -x RUBASH_EXPORT_UNSET\n"));
    assert!(!output.contains("RUBASH_EXPORT_UNSET=\"\""));
    std::env::remove_var("RUBASH_EXPORT_UNSET");
    let _ = fs::remove_file(&output_path);
}
