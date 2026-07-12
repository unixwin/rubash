use super::super::*;
use std::fs;

#[test]
fn test_disabled_builtin_dispatch_uses_external_commands() {
    let bin_dir = "target/rubash-disabled-builtin-dispatch-bin";
    let outputs = [
        "target/rubash-disabled-cd-output.txt",
        "target/rubash-disabled-alias-output.txt",
        "target/rubash-disabled-type-output.txt",
        "target/rubash-disabled-command-cd-output.txt",
        "target/rubash-disabled-command-alias-output.txt",
        "target/rubash-disabled-command-type-output.txt",
        "target/rubash-disabled-command-builtin-output.txt",
    ];
    let _ = fs::remove_dir_all(bin_dir);
    for path in outputs {
        let _ = fs::remove_file(path);
    }
    fs::create_dir_all(bin_dir).unwrap();
    for name in ["cd", "alias", "type", "command"] {
        write_executable(
            format!("{bin_dir}/{name}"),
            format!("echo external-{name}\n"),
        )
        .unwrap();
    }
    let input = "\
        enable -n cd alias type; \
        cd > target/rubash-disabled-cd-output.txt; \
        alias > target/rubash-disabled-alias-output.txt; \
        type echo > target/rubash-disabled-type-output.txt; \
        command cd > target/rubash-disabled-command-cd-output.txt; \
        command alias > target/rubash-disabled-command-alias-output.txt; \
        command type echo > target/rubash-disabled-command-type-output.txt; \
        enable -n command; \
        command echo > target/rubash-disabled-command-builtin-output.txt";
    let tokens = tokenize(input);
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
        ("target/rubash-disabled-cd-output.txt", "external-cd\n"),
        (
            "target/rubash-disabled-alias-output.txt",
            "external-alias\n",
        ),
        ("target/rubash-disabled-type-output.txt", "external-type\n"),
        (
            "target/rubash-disabled-command-cd-output.txt",
            "external-cd\n",
        ),
        (
            "target/rubash-disabled-command-alias-output.txt",
            "external-alias\n",
        ),
        (
            "target/rubash-disabled-command-type-output.txt",
            "external-type\n",
        ),
        (
            "target/rubash-disabled-command-builtin-output.txt",
            "external-command\n",
        ),
    ] {
        assert_eq!(fs::read_to_string(path).unwrap(), expected);
        let _ = fs::remove_file(path);
    }
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn test_enable_appends_output() {
    let output_path = "target/rubash-enable-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("enable -ps >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("before\n"));
    assert!(output.contains("enable break\n"));
    assert!(output.contains("enable times\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_enable_redirects_stderr() {
    let error_path = "target/rubash-enable-stderr-output.txt";
    let status_path = "target/rubash-enable-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("enable no_such_builtin 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("enable: no_such_builtin: not a shell builtin"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_enable_appends_stderr() {
    let error_path = "target/rubash-enable-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("enable no_such_builtin 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("enable: no_such_builtin: not a shell builtin"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_builtin_echo_redirects_output() {
    let output_path = "target/rubash-builtin-echo-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin echo hello > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hello\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_echo_appends_output() {
    let output_path = "target/rubash-builtin-echo-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("builtin echo hello >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "before\nhello\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_missing_redirects_stderr() {
    let output_path = "target/rubash-builtin-missing-status.txt";
    let error_path = "target/rubash-builtin-missing-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("builtin no_such_builtin 2> {error_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("builtin: no_such_builtin: not a shell builtin"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_builtin_colon_redirect_truncates_output_file() {
    let output_path = "target/rubash-builtin-colon-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("builtin : > {output_path}; echo $? >> {output_path}");
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
fn test_builtin_type_invokes_type_builtin() {
    let output_path = "target/rubash-builtin-type-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("function type {{ echo function-type; }}; builtin type -t echo > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "builtin\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_test_invokes_test_builtin() {
    let output_path = "target/rubash-builtin-test-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin test 3 -eq 3; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
}
