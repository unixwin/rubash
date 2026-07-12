use super::super::*;
use std::fs;

#[test]
fn test_disabled_pwd_builtin_uses_external_command() {
    let bin_dir = "target/rubash-disabled-pwd-bin";
    let script_path = format!("{bin_dir}/pwd");
    let output_path = "target/rubash-disabled-pwd-output.txt";
    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    write_executable(&script_path, "echo external-pwd\n").unwrap();
    let input = format!("enable -n pwd; pwd > {output_path}; enable pwd");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    let old_path = std::env::var("PATH").ok();
    executor.set_env("PATH", bin_dir);

    let result = executor.execute_ast(&ast);
    match old_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "external-pwd\n");
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir(bin_dir);
}

#[test]
fn test_command_uses_external_pwd_when_builtin_is_disabled() {
    let bin_dir = "target/rubash-disabled-command-pwd-bin";
    let script_path = format!("{bin_dir}/pwd");
    let output_path = "target/rubash-disabled-command-pwd-output.txt";
    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    write_executable(&script_path, "echo external-command-pwd\n").unwrap();
    let input = format!("enable -n pwd; command pwd > {output_path}; enable pwd");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    let old_path = std::env::var("PATH").ok();
    executor.set_env("PATH", bin_dir);

    let result = executor.execute_ast(&ast);
    match old_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "external-command-pwd\n"
    );
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir(bin_dir);
}

#[test]
fn test_command_cd_updates_pwd_for_physical_pwd() {
    let output_path = "target/rubash-command-cd-physical-pwd-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("command cd -P /; command pwd -P > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    let old_pwd = std::env::var_os("PWD");

    let result = executor.execute_ast(&ast);
    match old_pwd {
        Some(value) => std::env::set_var("PWD", value),
        None => std::env::remove_var("PWD"),
    }

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "/\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_disabled_status_and_state_builtins_use_external_commands() {
    let bin_dir = "target/rubash-disabled-status-builtins-bin";
    let _ = fs::remove_dir_all(bin_dir);
    for path in [
        "target/rubash-disabled-true-output.txt",
        "target/rubash-disabled-false-output.txt",
        "target/rubash-disabled-hash-output.txt",
        "target/rubash-disabled-umask-output.txt",
        "target/rubash-disabled-command-true-output.txt",
        "target/rubash-disabled-command-false-output.txt",
        "target/rubash-disabled-command-hash-output.txt",
        "target/rubash-disabled-command-umask-output.txt",
    ] {
        let _ = fs::remove_file(path);
    }
    fs::create_dir_all(bin_dir).unwrap();
    for name in ["true", "false", "hash", "umask"] {
        write_executable(
            format!("{bin_dir}/{name}"),
            format!("echo external-{name}\n"),
        )
        .unwrap();
    }
    let input = format!(
        "enable -n true false hash umask; \
         true > target/rubash-disabled-true-output.txt; \
         false > target/rubash-disabled-false-output.txt; \
         hash > target/rubash-disabled-hash-output.txt; \
         umask > target/rubash-disabled-umask-output.txt; \
         command true > target/rubash-disabled-command-true-output.txt; \
         command false > target/rubash-disabled-command-false-output.txt; \
         command hash > target/rubash-disabled-command-hash-output.txt; \
         command umask > target/rubash-disabled-command-umask-output.txt; \
         enable true false hash umask"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    let old_path = std::env::var("PATH").ok();
    executor.set_env("PATH", bin_dir);

    let result = executor.execute_ast(&ast);
    match old_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    for (path, expected) in [
        ("target/rubash-disabled-true-output.txt", "external-true\n"),
        (
            "target/rubash-disabled-false-output.txt",
            "external-false\n",
        ),
        ("target/rubash-disabled-hash-output.txt", "external-hash\n"),
        (
            "target/rubash-disabled-umask-output.txt",
            "external-umask\n",
        ),
        (
            "target/rubash-disabled-command-true-output.txt",
            "external-true\n",
        ),
        (
            "target/rubash-disabled-command-false-output.txt",
            "external-false\n",
        ),
        (
            "target/rubash-disabled-command-hash-output.txt",
            "external-hash\n",
        ),
        (
            "target/rubash-disabled-command-umask-output.txt",
            "external-umask\n",
        ),
    ] {
        assert_eq!(fs::read_to_string(path).unwrap(), expected);
        let _ = fs::remove_file(path);
    }
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn test_builtin_command_respects_disabled_builtin_state() {
    let output_path = "target/rubash-disabled-builtin-direct-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "enable -n true hash; \
         builtin true > {output_path}; \
         echo true:$? >> {output_path}; \
         builtin hash >> {output_path}; \
         echo hash:$? >> {output_path}; \
         enable true hash"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "true:1\nhash:1\n");
    let _ = fs::remove_file(output_path);
}
