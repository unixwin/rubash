use super::super::*;
use std::fs;

#[test]
fn test_declare_plus_fx_clears_exported_function_attribute() {
    let output_path = "target/rubash-declare-clear-export-function-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("foo() {{ :; }}; export -f foo; declare +fx foo; declare -xF > {output_path}");
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
fn test_export_f_missing_function_reports_error() {
    let error_path = "target/rubash-export-f-missing-error.txt";
    let status_path = "target/rubash-export-f-missing-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("export -f missing 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("export: missing: not a function"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_bash_func_environment_import_defines_function() {
    let output_path = "target/rubash-imported-function-output.txt";
    let _ = fs::remove_file(output_path);
    let old_value = std::env::var("BASH_FUNC_rubash_imported%%").ok();
    std::env::set_var("BASH_FUNC_rubash_imported%%", "() { echo imported; }");
    let input = format!("declare -F rubash_imported > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    match old_value {
        Some(value) => std::env::set_var("BASH_FUNC_rubash_imported%%", value),
        None => std::env::remove_var("BASH_FUNC_rubash_imported%%"),
    }

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "rubash_imported\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_export_f_makes_function_available_to_child_rubash() {
    let output_path = target_test_path("rubash-exported-function-child-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let input = format!(
        "rubash_child_func() {{ echo child; }}; export -f rubash_child_func; \
         {rubash} -c rubash_child_func > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "child\n");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_export_f_preserves_function_heredoc_in_child_rubash() {
    let output_path = target_test_path("rubash-exported-function-heredoc-child-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let input = format!(
        "rubash_heredoc_func()\n{{\ncat <<EOF > /dev/null\nbody\nEOF\naa=1\n}}\n\
         export -f rubash_heredoc_func; \
         {rubash} -c 'type rubash_heredoc_func' > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "rubash_heredoc_func is a function\nrubash_heredoc_func () \n{ \n    cat <<EOF > /dev/null\nbody\nEOF\n\n    aa=1\n}\n"
    );
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_function_call_redirects_entire_body_output() {
    let output_path = "target/rubash-function-call-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "rubash_redirect_func() {{ echo one; echo two; }}; \
         rubash_redirect_func > {output_path}; echo done >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "one\ntwo\ndone\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_definition_redirect_applies_when_called() {
    let output_path = target_test_path("rubash-function-definition-redirect-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "rubash_def_redirect_func() {{ echo one; echo two; }} > {shell_output_path}; \
         test -e {shell_output_path}; echo before:$?; \
         rubash_def_redirect_func; rubash_def_redirect_func"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "one\ntwo\n");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_compact_function_definition_redirect_applies_when_called() {
    let output_path = target_test_path("rubash-compact-function-definition-redirect-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "rubash_compact_def_redirect_func(){{ echo compact; }} > {shell_output_path}; \
         rubash_compact_def_redirect_func"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "compact\n");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_short_function_definition_redirect_applies_when_called() {
    let output_path = "target/rubash-short-function-definition-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("f(){{ echo hi; }} > {output_path}; f");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hi\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_definition_stdin_redirect_overrides_call_stdin() {
    let definition_input = target_test_path("rubash-function-definition-stdin-input.txt");
    let call_input = target_test_path("rubash-function-call-stdin-override-input.txt");
    let output_path = target_test_path("rubash-function-definition-stdin-output.txt");
    let shell_definition_input = shell_test_path(&definition_input);
    let shell_call_input = shell_test_path(&call_input);
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&definition_input);
    let _ = fs::remove_file(&call_input);
    let _ = fs::remove_file(&output_path);
    fs::write(&definition_input, "definition\n").unwrap();
    fs::write(&call_input, "call\n").unwrap();
    let input = format!(
        "rubash_def_stdin_func() {{ read line; echo $line; }} < {shell_definition_input}; \
         rubash_def_stdin_func < {shell_call_input} > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "definition\n");
    let _ = fs::remove_file(&definition_input);
    let _ = fs::remove_file(&call_input);
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_function_call_redirects_stdin_to_body_reads() {
    let input_path = target_test_path("rubash-function-call-stdin-input.txt");
    let output_path = target_test_path("rubash-function-call-stdin-output.txt");
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);
    fs::write(&input_path, "alpha\nbeta\n").unwrap();
    let shell_input_path = shell_test_path(&input_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "rubash_stdin_func() {{ read first; read second; echo $first/$second; }}; \
         rubash_stdin_func < {shell_input_path} > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "alpha/beta\n");
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);
}
