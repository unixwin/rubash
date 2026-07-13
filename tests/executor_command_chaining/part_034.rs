use super::super::*;
use std::fs;

#[test]
fn test_source_option_errors_return_usage_status() {
    let output_path = "target/rubash-source-option-errors-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "source; echo nofile:$? > {output_path}; \
         source -p; echo missingpath:$? >> {output_path}; \
         . -i; echo invalid:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "nofile:2\nmissingpath:2\ninvalid:2\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_source_option_errors_redirect_stderr() {
    let output_path = "target/rubash-source-option-errors-redirect-status.txt";
    let error_path = "target/rubash-source-option-errors-redirect-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "source 2> {error_path}; echo missing:$? > {output_path}; \
         . -i 2>> {error_path}; echo invalid:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "missing:2\ninvalid:2\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("source: filename argument required"));
    assert!(error.contains("source: usage: source"));
    assert!(error.contains(".: -i: invalid option"));
    assert!(error.contains(".: usage: ."));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_source_missing_file_redirects_stderr() {
    let output_path = "target/rubash-source-missing-redirect-status.txt";
    let error_path = "target/rubash-source-missing-redirect-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("source no_such_source_file 2> {error_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("no_such_source_file: No such file or directory"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_source_redirects_body_stdout_and_stderr() {
    let script_path = "target/rubash-source-body-redirect.sh";
    let output_path = "target/rubash-source-body-redirect-output.txt";
    let error_path = "target/rubash-source-body-redirect-error.txt";
    let status_path = "target/rubash-source-body-redirect-status.txt";
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    fs::write(script_path, "echo source-out\nno_such_source_body_cmd\n").unwrap();
    let input =
        format!("source {script_path} > {output_path} 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "127\n");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "source-out\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("no_such_source_body_cmd: command not found"));
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_source_appends_body_stdout_and_stderr() {
    let script_path = "target/rubash-source-body-append.sh";
    let output_path = "target/rubash-source-body-append-output.txt";
    let error_path = "target/rubash-source-body-append-error.txt";
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    fs::write(
        script_path,
        "echo appended-out\nno_such_source_append_cmd\n",
    )
    .unwrap();
    fs::write(output_path, "before-out\n").unwrap();
    fs::write(error_path, "before-err\n").unwrap();
    let input = format!("source {script_path} >> {output_path} 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 127);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before-out\nappended-out\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before-err\n"));
    assert!(error.contains("no_such_source_append_cmd: command not found"));
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_source_process_substitution_updates_current_shell() {
    let output_path = "target/rubash-source-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "source <(printf 'RUBASH_SOURCE_PS=ok\\n'); echo $RUBASH_SOURCE_PS > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_source_process_substitution_uses_source_arguments() {
    let output_path = "target/rubash-source-process-substitution-args-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("source <(printf 'echo sourced:$1 > {output_path}\\n') arg");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "sourced:arg\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_return_returns_from_function() {
    let output_path = "target/rubash-builtin-return-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() {{ builtin return 6; echo bad > {output_path}; }}; f; echo $? > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "6\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_caller_top_level_returns_failure() {
    let output_path = "target/rubash-caller-top-output.txt";
    let status_path = "target/rubash-caller-top-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("caller > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_caller_reports_current_function_call_site() {
    let output_path = "target/rubash-caller-current-output.txt";
    let status_path = "target/rubash-caller-current-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("f() {{ caller > {output_path}; echo $? > {status_path}; }}; f");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1 NULL\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_caller_zero_reports_parent_function_frame() {
    let output_path = "target/rubash-caller-zero-output.txt";
    let status_path = "target/rubash-caller-zero-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input =
        format!("f() {{ caller 0 > {output_path}; echo $? > {status_path}; }}; g() {{ f; }}; g");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "1 g environment\n"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_caller_invalid_argument_returns_usage() {
    let error_path = "target/rubash-caller-invalid-error.txt";
    let status_path = "target/rubash-caller-invalid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("f() {{ caller nope 2> {error_path}; echo $? > {status_path}; }}; f");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("caller: nope: invalid number"));
    assert!(error.contains("caller: usage: caller [expr]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}
