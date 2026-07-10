use super::super::*;
use std::fs;

#[test]
fn test_export_assignment_rejects_readonly_variable() {
    let output_path = target_test_path("rubash-export-readonly-assignment-output.txt");
    let error_path = target_test_path("rubash-export-readonly-assignment-error.txt");
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let input = format!(
        "readonly RUBASH_EXPORT_READONLY_ASSIGN=1; \
         export RUBASH_EXPORT_READONLY_ASSIGN=2 2> {shell_error_path}; echo $? > {shell_output_path}; \
         declare -p RUBASH_EXPORT_READONLY_ASSIGN >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "1\ndeclare -r RUBASH_EXPORT_READONLY_ASSIGN=\"1\"\n"
    );
    let error = fs::read_to_string(&error_path).unwrap();
    assert!(error.contains("RUBASH_EXPORT_READONLY_ASSIGN: readonly variable"));
    std::env::remove_var("RUBASH_EXPORT_READONLY_ASSIGN");
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
}

#[test]
fn test_export_without_assignment_marks_readonly_variable() {
    let output_path = target_test_path("rubash-export-readonly-unset-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "unset RUBASH_EXPORT_READONLY_UNSET; readonly RUBASH_EXPORT_READONLY_UNSET; \
         export RUBASH_EXPORT_READONLY_UNSET; declare -p RUBASH_EXPORT_READONLY_UNSET > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "declare -rx RUBASH_EXPORT_READONLY_UNSET\n"
    );
    std::env::remove_var("RUBASH_EXPORT_READONLY_UNSET");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_attribute_printing_preserves_combined_flags() {
    let output_path = target_test_path("rubash-attribute-combined-flags-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "declare -ir RUBASH_READONLY_INT=7; readonly -p > {shell_output_path}; \
         export RUBASH_READONLY_INT; export -p >> {shell_output_path}; \
         declare -ux RUBASH_EXPORT_UPPER=abc; export -p >> {shell_output_path}; \
         declare -lrx RUBASH_READONLY_LOWER=ABC; readonly -p >> {shell_output_path}; export -p >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.contains("declare -ir RUBASH_READONLY_INT=\"7\"\n"));
    assert!(output.contains("declare -irx RUBASH_READONLY_INT=\"7\"\n"));
    assert!(output.contains("declare -xu RUBASH_EXPORT_UPPER=\"ABC\"\n"));
    assert!(output.contains("declare -rxl RUBASH_READONLY_LOWER=\"abc\"\n"));
    std::env::remove_var("RUBASH_READONLY_INT");
    std::env::remove_var("RUBASH_EXPORT_UPPER");
    std::env::remove_var("RUBASH_READONLY_LOWER");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_child_environment_contains_only_exported_variables() {
    let output_path = target_test_path("rubash-child-export-env-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let input = format!(
        "RUBASH_CHILD_LOCAL=local; export RUBASH_CHILD_EXPORTED=exported; \
         {rubash} -c 'printf \"%s/%s\\n\" \"${{RUBASH_CHILD_LOCAL-unset}}\" \"$RUBASH_CHILD_EXPORTED\"' > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "unset/exported\n"
    );
    std::env::remove_var("RUBASH_CHILD_LOCAL");
    std::env::remove_var("RUBASH_CHILD_EXPORTED");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_temporary_assignment_reaches_external_command_environment() {
    let output_path = target_test_path("rubash-temp-assignment-env-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let input = format!(
        "RUBASH_TEMP_ENV=command {rubash} -c 'printf \"%s\\n\" \"$RUBASH_TEMP_ENV\"' > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "command\n");
    std::env::remove_var("RUBASH_TEMP_ENV");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_temporary_assignment_reaches_shell_function() {
    let output_path = target_test_path("rubash-temp-assignment-function-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "RUBASH_TEMP_FUNC=outer; \
         f() {{ printf '%s\\n' \"$RUBASH_TEMP_FUNC\"; RUBASH_TEMP_FUNC=changed; }}; \
         RUBASH_TEMP_FUNC=temp f > {shell_output_path}; \
         printf '%s\\n' \"$RUBASH_TEMP_FUNC\" >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "temp\nouter\n");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_posix_function_temporary_assignment_persists_when_exported() {
    let output_path = target_test_path("rubash-posix-temp-function-export-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "set -o posix; var=outside; \
         f() {{ export var; printf 'inside:%s\\n' \"${{var-<unset>}}\" > {shell_output_path}; }}; \
         var=func f; printf 'outside:%s\\n' \"${{var-<unset>}}\" >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "inside:func\noutside:func\n"
    );
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_posix_function_declare_prefix_assignment_stays_local() {
    let output_path = target_test_path("rubash-posix-declare-prefix-local-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "set -o posix; var=one; \
         f() {{ var=value declare -x var; echo inside:$var > {shell_output_path}; }}; \
         f; echo outside:$var >> {shell_output_path}; declare -p var >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "inside:value\noutside:one\ndeclare -- var=\"one\"\n"
    );
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_posix_function_typeset_plus_x_unsets_shell_value() {
    let output_path = target_test_path("rubash-posix-typeset-plus-x-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "set -o posix; \
         f() {{ foo=abc; export foo; typeset +x foo; \
                echo shell:${{foo-unset}} > {shell_output_path}; \
                declare -p foo >> {shell_output_path}; }}; f"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "shell:unset\ndeclare -- foo\n"
    );
    let _ = fs::remove_file(&output_path);
}
