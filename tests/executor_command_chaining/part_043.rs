use super::super::*;
use std::fs;

#[test]
fn test_type_invalid_option_redirects_stderr() {
    let status_path = "target/rubash-type-invalid-option-status.txt";
    let error_path = "target/rubash-type-invalid-option-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!("type -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("type: -Z: invalid option"));
    assert!(error.contains("type: usage: type"));
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_type_missing_name_redirects_stderr() {
    let status_path = "target/rubash-type-missing-name-status.txt";
    let error_path = "target/rubash-type-missing-name-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!("type no_such_name 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("type: no_such_name: not found"));
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_command_v_redirects_output() {
    let output_path = "target/rubash-command-v-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("command -v echo > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "echo\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_invalid_option_redirects_stderr() {
    let status_path = "target/rubash-command-invalid-option-status.txt";
    let error_path = "target/rubash-command-invalid-option-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!("command -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("command: -Z: invalid option"));
    assert!(error.contains("command: usage: command"));
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_command_verbose_missing_redirects_stderr() {
    let status_path = "target/rubash-command-verbose-missing-status.txt";
    let error_path = "target/rubash-command-verbose-missing-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!("command -V no_such_name 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("command: no_such_name: not found"));
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_command_p_uses_standard_path_for_external_command() {
    let output_path = "target/rubash-command-p-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("PATH=target/rubash-no-such-bin command -p sh -c 'echo ok' > {output_path}");
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
fn test_command_p_v_uses_standard_path_for_external_command() {
    let output_path = "target/rubash-command-p-v-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("PATH=target/rubash-no-such-bin command -p -v sh > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.trim_end().ends_with("sh") || output.trim_end().ends_with("sh.exe"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_v_without_p_uses_current_path_for_external_command() {
    let bin_dir = target_test_path("rubash-command-v-bin");
    let missing_bin_dir = target_test_path("rubash-no-such-bin");
    #[cfg(windows)]
    let script_path = bin_dir.join("sh.cmd");
    #[cfg(not(windows))]
    let script_path = bin_dir.join("sh");
    let status_path = target_test_path("rubash-command-v-without-p-status.txt");
    let output_path = target_test_path("rubash-command-v-without-p-output.txt");
    let restored_path = target_test_path("rubash-command-v-restored-output.txt");
    let restored_status_path = target_test_path("rubash-command-v-restored-status.txt");
    let shell_bin_dir = shell_test_path(&bin_dir);
    let shell_missing_bin_dir = shell_test_path(&missing_bin_dir);
    let shell_status_path = shell_test_path(&status_path);
    let shell_output_path = shell_test_path(&output_path);
    let shell_restored_path = shell_test_path(&restored_path);
    let shell_restored_status_path = shell_test_path(&restored_status_path);
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_dir_all(&missing_bin_dir);
    let _ = fs::remove_file(&status_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&restored_path);
    let _ = fs::remove_file(&restored_status_path);
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(&script_path, "echo fake-sh\n").unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
    }
    let input = format!(
        "PATH={shell_missing_bin_dir} command -v sh > {shell_output_path}; \
         echo $? > {shell_status_path}; command -v sh > {shell_restored_path}; \
         echo $? > {shell_restored_status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("PATH", &shell_bin_dir);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "");
    assert_eq!(fs::read_to_string(&status_path).unwrap(), "1\n");
    assert!(!fs::read_to_string(&restored_path).unwrap().is_empty());
    assert_eq!(fs::read_to_string(&restored_status_path).unwrap(), "0\n");
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(restored_path);
    let _ = fs::remove_file(restored_status_path);
    let _ = fs::remove_dir_all(bin_dir);
}
