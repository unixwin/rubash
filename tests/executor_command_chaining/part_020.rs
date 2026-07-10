use super::super::*;
use std::fs;

#[test]
fn test_backtick_in_comment_does_not_swallow_temporary_export() {
    let output_path = target_test_path("rubash-comment-backtick-temp-export-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "# assignment before `eval' and `.'\n\
         export RUBASH_COMMENT_TEMP=old\n\
         export -n RUBASH_COMMENT_TEMP # make sure it's not exported\n\
         echo expect new > {shell_output_path}\n\
         RUBASH_COMMENT_TEMP=new export RUBASH_COMMENT_TEMP\n\
         declare -p RUBASH_COMMENT_TEMP >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "expect new\ndeclare -x RUBASH_COMMENT_TEMP=\"new\"\n"
    );
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_child_environment_includes_exported_pwd() {
    let output_path = target_test_path("rubash-child-pwd-env-output.txt");
    #[cfg(windows)]
    let script_path = target_test_path("rubash-child-pwd-env.cmd");
    #[cfg(not(windows))]
    let script_path = target_test_path("rubash-child-pwd-env.sh");
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&script_path);
    let shell_output_path = shell_test_path(&output_path);
    let shell_script_path = shell_test_path(&script_path);
    #[cfg(windows)]
    fs::write(
        &script_path,
        "@echo off\r\nif \"%PWD%\"==\"\" (echo unset) else echo %PWD%\r\n",
    )
    .unwrap();
    #[cfg(not(windows))]
    fs::write(&script_path, "#!/bin/sh\nprintf '%s\\n' \"${PWD-unset}\"\n").unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
    }
    let input = format!("{shell_script_path} > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path)
        .unwrap()
        .replace("\r\n", "\n");
    assert_ne!(output, "unset\n");
    assert!(!output.trim().is_empty());
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&script_path);
}

#[test]
fn test_initial_oldpwd_is_exported_but_unset() {
    let output_path = target_test_path("rubash-initial-oldpwd-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let original_oldpwd = std::env::var("OLDPWD").ok();
    std::env::set_var("OLDPWD", "/tmp/parent-oldpwd");
    let input = format!(
        "printf '<%s>\\n' \"${{OLDPWD-unset}}\" > {shell_output_path}; \
         declare -p OLDPWD >> {shell_output_path}; export -p >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    match original_oldpwd {
        Some(value) => std::env::set_var("OLDPWD", value),
        None => std::env::remove_var("OLDPWD"),
    }
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.starts_with("<unset>\ndeclare -x OLDPWD\n"));
    assert!(output.contains("declare -x OLDPWD\n"));
    assert!(!output.contains("parent-oldpwd"));
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_shell_level_increments_inherited_environment() {
    let output_path = target_test_path("rubash-shlvl-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let original_shlvl = std::env::var("SHLVL").ok();
    std::env::set_var("SHLVL", "7");
    let input = format!(
        "printf '%s\\n' \"$SHLVL\" > {shell_output_path}; declare -p SHLVL >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    match original_shlvl {
        Some(value) => std::env::set_var("SHLVL", value),
        None => std::env::remove_var("SHLVL"),
    }
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "8\ndeclare -x SHLVL=\"8\"\n"
    );
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_child_rubash_increments_shell_level() {
    let output_path = target_test_path("rubash-child-shlvl-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let input = format!("SHLVL=12 {rubash} -c 'printf \"%s\\n\" \"$SHLVL\"' > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "13\n");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_readonly_p_redirects_output() {
    let output_path = "target/rubash-readonly-p-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("readonly RUBASH_READONLY_REDIR=value; readonly -p > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("declare -r RUBASH_READONLY_REDIR=\"value\"\n"));
    assert!(output.contains("declare -r SHELLOPTS=\""));
    std::env::remove_var("RUBASH_READONLY_REDIR");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_readonly_without_assignment_marks_unset_variable() {
    let output_path = target_test_path("rubash-readonly-unset-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "unset RUBASH_READONLY_UNSET; readonly RUBASH_READONLY_UNSET; \
         printf '<%s>\\n' \"${{RUBASH_READONLY_UNSET-unset}}\" > {shell_output_path}; \
         readonly -p >> {shell_output_path}; \
         printf '%s\\n' --- >> {shell_output_path}; \
         declare -p RUBASH_READONLY_UNSET >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.starts_with("<unset>\n"));
    assert!(output.contains("declare -r RUBASH_READONLY_UNSET\n"));
    assert!(output.ends_with("---\ndeclare -r RUBASH_READONLY_UNSET\n"));
    assert!(!output.contains("RUBASH_READONLY_UNSET=\"\""));
    std::env::remove_var("RUBASH_READONLY_UNSET");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_builtin_export_assigns_variable() {
    let output_path = "target/rubash-builtin-export-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "builtin export RUBASH_BUILTIN_EXPORT=value; echo $RUBASH_BUILTIN_EXPORT > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "value\n");
    std::env::remove_var("RUBASH_BUILTIN_EXPORT");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_readonly_assigns_variable() {
    let output_path = "target/rubash-builtin-readonly-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin readonly RUBASH_BUILTIN_READONLY=value; echo $RUBASH_BUILTIN_READONLY > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "value\n");
    std::env::remove_var("RUBASH_BUILTIN_READONLY");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_export_redirects_stderr() {
    let error_path = "target/rubash-export-stderr-output.txt";
    let status_path = "target/rubash-export-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("export -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("rubash: export: -Z: invalid option"));
    assert!(error.contains("export: usage:"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}
