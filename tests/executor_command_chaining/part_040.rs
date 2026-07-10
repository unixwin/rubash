use super::super::*;
use std::fs;

#[test]
fn test_command_source_redirects_body_stdout_and_stderr() {
    let script_path = "target/rubash-command-source-body-redirect.sh";
    let output_path = "target/rubash-command-source-body-output.txt";
    let error_path = "target/rubash-command-source-body-error.txt";
    let status_path = "target/rubash-command-source-body-status.txt";
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    fs::write(
        script_path,
        "echo command-source-out\nno_such_command_source_body\n",
    )
    .unwrap();
    let input = format!(
        "command source {script_path} > {output_path} 2> {error_path}; echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "127\n");
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "command-source-out\n"
    );
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("no_such_command_source_body: command not found"));
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_command_source_searches_sourcepath() {
    let bin_dir = "target/rubash-command-sourcepath-bin";
    let script_name = "rubash-command-sourcepath-script.sh";
    let script_path = format!("{bin_dir}/{script_name}");
    let output_path = "target/rubash-command-sourcepath-output.txt";
    let _ = fs::remove_dir_all(bin_dir);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    fs::write(&script_path, "echo command-sourcepath\n").unwrap();
    let input = format!("PATH={bin_dir}; command source {script_name} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "command-sourcepath\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn test_command_dot_missing_redirects_stderr() {
    let output_path = "target/rubash-command-dot-missing-status.txt";
    let error_path = "target/rubash-command-dot-missing-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("command . no_such_dot_file 2> {error_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("no_such_dot_file: No such file or directory"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_command_dot_appends_body_stdout_and_stderr() {
    let script_path = "target/rubash-command-dot-body-append.sh";
    let output_path = "target/rubash-command-dot-body-output.txt";
    let error_path = "target/rubash-command-dot-body-error.txt";
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    fs::write(
        script_path,
        "echo command-dot-out\nno_such_command_dot_body\n",
    )
    .unwrap();
    fs::write(output_path, "before-out\n").unwrap();
    fs::write(error_path, "before-err\n").unwrap();
    let input = format!("command . {script_path} >> {output_path} 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 127);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before-out\ncommand-dot-out\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before-err\n"));
    assert!(error.contains("no_such_command_dot_body: command not found"));
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_command_declare_assigns_variable() {
    let output_path = "target/rubash-command-declare-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "command declare RUBASH_COMMAND_DECLARE=value; echo $RUBASH_COMMAND_DECLARE > {output_path}"
    );
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
fn test_command_unset_removes_variable() {
    let output_path = "target/rubash-command-unset-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "RUBASH_COMMAND_UNSET=value; command unset RUBASH_COMMAND_UNSET; echo ${{RUBASH_COMMAND_UNSET:-missing}} > {output_path}"
    );
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
fn test_command_shopt_updates_shell_option_state() {
    let output_path = "target/rubash-command-shopt-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("command shopt -s nullglob; shopt -q nullglob; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_eval_redirects_output() {
    let output_path = "target/rubash-command-eval-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("command eval 'echo alpha; echo beta' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_exec_redirects_output() {
    let output_path = "target/rubash-command-exec-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("command exec -a custom sh -c 'echo $0' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "custom\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shift_help_redirects_output_and_returns_usage() {
    let output_path = "target/rubash-shift-help-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("shift --help > {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("shift: shift [n]\n"));
    assert!(output.contains("Shift positional parameters."));
    assert!(output.ends_with("2\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shift_help_appends_output_and_returns_usage() {
    let output_path = "target/rubash-shift-help-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("shift --help >> {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("before\nshift: shift [n]\n"));
    assert!(output.contains("Shift positional parameters."));
    assert!(output.ends_with("2\n"));
    let _ = fs::remove_file(output_path);
}
