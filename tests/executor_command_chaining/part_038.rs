use super::super::*;
use std::fs;

#[test]
fn test_exec_a_accepts_attached_argument() {
    let output_path = "target/rubash-exec-a-attached-argument-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("exec -acustom sh -c 'echo $0' > {output_path}");
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
fn test_exec_double_dash_stops_option_parsing() {
    let output_path = "target/rubash-exec-double-dash-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("exec -- sh -c 'echo $0' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "sh\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_stdout_redirect_persists_for_following_commands() {
    let output_path = target_test_path("rubash-exec-persistent-stdout-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "exec > {shell_output_path}; value=$(echo captured); echo first; echo value:$value"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "first\nvalue:captured\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_stderr_redirect_persists_for_following_commands() {
    let error_path = target_test_path("rubash-exec-persistent-stderr-output.txt");
    let shell_error_path = shell_test_path(&error_path);
    let _ = fs::remove_file(&error_path);
    let input = format!(
        "exec 2> {shell_error_path}; builtin no_such_rubash_builtin_one; builtin no_such_rubash_builtin_two"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(&error_path).unwrap();
    assert!(error.contains("builtin: no_such_rubash_builtin_one: not a shell builtin"));
    assert!(error.contains("builtin: no_such_rubash_builtin_two: not a shell builtin"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_exec_stdin_redirect_persists_for_following_reads() {
    let input_path = target_test_path("rubash-exec-persistent-stdin-input.txt");
    let output_path = target_test_path("rubash-exec-persistent-stdin-output.txt");
    let shell_input_path = shell_test_path(&input_path);
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);
    fs::write(&input_path, "first\nsecond\n").unwrap();
    let input =
        format!("exec < {shell_input_path}; read a; read b; echo $a/$b > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "first/second\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_stdout_redirect_is_inherited_by_external_commands() {
    let output_path = target_test_path("rubash-exec-external-stdout-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let _ = fs::remove_file(&output_path);
    let input = format!("exec > {shell_output_path}; {rubash} -c 'echo external-out'");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "external-out\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_stderr_redirect_is_inherited_by_external_commands() {
    let error_path = target_test_path("rubash-exec-external-stderr-output.txt");
    let shell_error_path = shell_test_path(&error_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let _ = fs::remove_file(&error_path);
    let input =
        format!("exec 2> {shell_error_path}; {rubash} -c 'builtin no_such_rubash_external_err'");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(&error_path).unwrap();
    assert!(error.contains("builtin: no_such_rubash_external_err: not a shell builtin"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_exec_stdin_redirect_is_inherited_by_external_commands() {
    let input_path = target_test_path("rubash-exec-external-stdin-input.txt");
    let output_path = target_test_path("rubash-exec-external-stdin-output.txt");
    let shell_input_path = shell_test_path(&input_path);
    let shell_output_path = shell_test_path(&output_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);
    fs::write(&input_path, "external-in\n").unwrap();
    let input =
        format!("exec < {shell_input_path}; {rubash} -c 'read x; echo $x' > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "external-in\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_child_environment_contains_only_exported_variables() {
    let output_path = target_test_path("rubash-exec-child-env-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let input = format!(
        "RUBASH_EXEC_LOCAL=local; export RUBASH_EXEC_EXPORTED=exported; \
         exec {rubash} -c 'printf \"%s/%s\\n\" \"${{RUBASH_EXEC_LOCAL-unset}}\" \"$RUBASH_EXEC_EXPORTED\"' > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "unset/exported\n"
    );
    std::env::remove_var("RUBASH_EXEC_LOCAL");
    std::env::remove_var("RUBASH_EXEC_EXPORTED");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_exec_printenv_ignores_unexported_variable() {
    let output_path = target_test_path("rubash-exec-printenv-local-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!("RUBASH_EXEC_PRINTENV_LOCAL=local; exec printenv RUBASH_EXEC_PRINTENV_LOCAL > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
    assert_eq!(executor.last_exit_code(), 1);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "");
    std::env::remove_var("RUBASH_EXEC_PRINTENV_LOCAL");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_exec_printenv_reads_exported_variable() {
    let output_path = target_test_path("rubash-exec-printenv-exported-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!("export RUBASH_EXEC_PRINTENV_EXPORTED=exported; exec printenv RUBASH_EXEC_PRINTENV_EXPORTED > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "exported\n");
    std::env::remove_var("RUBASH_EXEC_PRINTENV_EXPORTED");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_exec_runs_external_command() {
    let output_path = target_test_path("rubash-exec-runs-external-output.txt");
    #[cfg(windows)]
    let script_path = target_test_path("rubash-exec-runs-external.cmd");
    #[cfg(not(windows))]
    let script_path = target_test_path("rubash-exec-runs-external.sh");
    let shell_output_path = shell_test_path(&output_path);
    let shell_script_path = shell_test_path(&script_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&script_path);
    #[cfg(windows)]
    fs::write(&script_path, "@echo off\r\necho exec-ran\r\n").unwrap();
    #[cfg(not(windows))]
    fs::write(&script_path, "#!/bin/sh\nprintf '%s\\n' exec-ran\n").unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
    }
    let input = format!("exec {shell_script_path} > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path)
            .unwrap()
            .replace("\r\n", "\n"),
        "exec-ran\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(script_path);
}

#[test]
fn test_exec_stops_after_successful_command() {
    let output_path = target_test_path("rubash-exec-stops-output.txt");
    #[cfg(windows)]
    let script_path = target_test_path("rubash-exec-stops.cmd");
    #[cfg(not(windows))]
    let script_path = target_test_path("rubash-exec-stops.sh");
    let shell_output_path = shell_test_path(&output_path);
    let shell_script_path = shell_test_path(&script_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&script_path);
    #[cfg(windows)]
    fs::write(&script_path, "@echo off\r\necho exec-only\r\n").unwrap();
    #[cfg(not(windows))]
    fs::write(&script_path, "#!/bin/sh\nprintf '%s\\n' exec-only\n").unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
    }
    let input = format!(
        "exec {shell_script_path} > {shell_output_path}; echo after >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path)
            .unwrap()
            .replace("\r\n", "\n"),
        "exec-only\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(script_path);
}

#[test]
fn test_exec_c_clears_external_command_environment() {
    let output_path = target_test_path("rubash-exec-clean-env-output.txt");
    let script_path = target_test_path("rubash-exec-clean-env.sh");
    #[cfg(windows)]
    let shell_path = std::path::PathBuf::from(
        std::env::var("CLAUDE_CODE_GIT_BASH_PATH")
            .unwrap_or_else(|_| r"D:\Git\bin\bash.exe".to_string()),
    );
    #[cfg(not(windows))]
    let shell_path = std::path::PathBuf::from("/bin/sh");
    let shell_output_path = shell_test_path(&output_path);
    let shell_script_path = shell_test_path(&script_path);
    let shell_command_path = shell_test_path(&shell_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&script_path);
    fs::write(
        &script_path,
        "if [ -n \"$FOO\" ]; then printf 'FOO=%s\\n' \"$FOO\"; else printf 'FOO=\\n'; fi\n",
    )
    .unwrap();

    let input = format!("exec -c {shell_command_path} {shell_script_path} > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("FOO", "present");

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "FOO=\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(script_path);
}

#[test]
fn test_exec_missing_command_returns_not_found() {
    let error_path = "target/rubash-exec-missing-command-error.txt";
    let _ = fs::remove_file(error_path);
    let input = format!("exec no_such_rubash_command 2> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(127))));
    assert_eq!(executor.last_exit_code(), 127);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("exec: no_such_rubash_command: not found"));
    let _ = fs::remove_file(error_path);
}
