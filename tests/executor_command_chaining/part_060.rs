use super::super::*;
use std::fs;

#[test]
fn test_remaining_stateful_set_short_flags_update_shell_options() {
    let output_path = "target/rubash-set-remaining-flags-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "echo $- > {output_path}; \
         [[ -o privileged ]]; echo $? >> {output_path}; \
         set -EHTpt; echo $- >> {output_path}; \
         [[ -o errtrace ]]; echo $? >> {output_path}; \
         [[ -o histexpand ]]; echo $? >> {output_path}; \
         [[ -o functrace ]]; echo $? >> {output_path}; \
         [[ -o privileged ]]; echo $? >> {output_path}; \
         [[ -o onecmd ]]; echo $? >> {output_path}; \
         set +BEHTpt; echo $- >> {output_path}; \
         [[ -o braceexpand ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let lines: Vec<String> = fs::read_to_string(output_path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert!(lines[0].contains('B'));
    for flag in ['E', 'H', 'T', 'p', 't'] {
        assert!(!lines[0].contains(flag));
    }
    assert_eq!(lines[1], "1");
    for flag in ['E', 'H', 'T', 'p', 't'] {
        assert!(lines[2].contains(flag));
    }
    assert_eq!(lines[3..8], ["0", "0", "0", "0", "0"].map(str::to_string));
    for flag in ['B', 'E', 'H', 'T', 'p', 't'] {
        assert!(!lines[8].contains(flag));
    }
    assert_eq!(lines[9], "1");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_noclobber_prevents_output_overwrite() {
    let output_path = "target/rubash-noclobber-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "old\n").unwrap();
    let input = format!("set -C; echo new > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::IoError(_))));
    assert_eq!(fs::read_to_string(output_path).unwrap(), "old\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_external_command_not_found_redirects_stderr() {
    let output_path = "target/rubash-external-notfound-status.txt";
    let error_path = "target/rubash-external-notfound-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("no_such_rubash_command 2> {error_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "127\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("no_such_rubash_command: command not found"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_command_not_found_redirects_stderr_and_creates_stdout_redirect() {
    let output_path = "target/rubash-command-notfound-output.txt";
    let status_path = "target/rubash-command-notfound-status.txt";
    let error_path = "target/rubash-command-notfound-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "command no_such_rubash_command > {output_path} 2> {error_path}; echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "127\n");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("no_such_rubash_command: command not found"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_noclobber_can_be_disabled_for_output_overwrite() {
    let output_path = "target/rubash-noclobber-disabled-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "old\n").unwrap();
    let input = format!("set -C; set +C; printf new > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "new");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_clobber_redirect_overrides_noclobber() {
    let output_path = "target/rubash-clobber-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "old\n").unwrap();
    let input = format!("set -C; echo new >| {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "new\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_noclobber_prevents_stderr_overwrite() {
    let error_path = "target/rubash-noclobber-stderr.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "old\n").unwrap();
    let input = format!("set -C; unalias no_such_alias 2> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::IoError(_))));
    assert_eq!(fs::read_to_string(error_path).unwrap(), "old\n");
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_stderr_clobber_redirect_overrides_noclobber() {
    let error_path = "target/rubash-clobber-stderr.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "old\n").unwrap();
    let input = format!("set -C; unalias no_such_alias 2>| {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let output = fs::read_to_string(error_path).unwrap();
    assert!(output.contains("no_such_alias"));
    assert!(!output.contains("old"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_nounset_errors_for_unbound_variable() {
    let output_path = "target/rubash-nounset-unbound-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset RUBASH_NOUNSET_MISSING; set -u; echo $RUBASH_NOUNSET_MISSING > {output_path}; echo after > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(127))));
    assert_eq!(executor.last_exit_code(), 127);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_nounset_errors_for_unbound_positional_parameter() {
    let output_path = "target/rubash-nounset-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -u; echo $1 > {output_path}; echo after > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(127))));
    assert_eq!(executor.last_exit_code(), 127);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_nounset_allows_default_parameter_expansion() {
    let output_path = "target/rubash-nounset-default-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("unset RUBASH_NOUNSET_DEFAULT; set -u; echo ${{RUBASH_NOUNSET_DEFAULT:-fallback}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "fallback\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_nounset_errors_for_unbound_assignment_value() {
    let output_path = "target/rubash-nounset-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset RUBASH_NOUNSET_ASSIGNMENT; set -u; value=$RUBASH_NOUNSET_ASSIGNMENT; echo after > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(127))));
    assert_eq!(executor.last_exit_code(), 127);
    assert!(!std::path::Path::new(output_path).exists());
}
