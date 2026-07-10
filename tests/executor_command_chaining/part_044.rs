use super::super::*;
use std::fs;

#[test]
fn test_describe_disabled_builtin_uses_path_command() {
    let bin_dir = "target/rubash-disabled-describe-bin";
    let script_path = format!("{bin_dir}/echo");
    let command_v_path = "target/rubash-disabled-command-v-output.txt";
    let command_v_verbose_path = "target/rubash-disabled-command-v-verbose-output.txt";
    let type_t_path = "target/rubash-disabled-type-t-output.txt";
    let type_verbose_path = "target/rubash-disabled-type-verbose-output.txt";
    let _ = fs::remove_dir_all(bin_dir);
    for path in [
        command_v_path,
        command_v_verbose_path,
        type_t_path,
        type_verbose_path,
    ] {
        let _ = fs::remove_file(path);
    }
    fs::create_dir_all(bin_dir).unwrap();
    fs::write(&script_path, "echo external-echo\n").unwrap();
    let input = format!(
        "enable -n echo; \
         command -v echo > {command_v_path}; \
         command -V echo > {command_v_verbose_path}; \
         type -t echo > {type_t_path}; \
         type echo > {type_verbose_path}; \
         enable echo"
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
    let command_v = fs::read_to_string(command_v_path).unwrap();
    let command_v_verbose = fs::read_to_string(command_v_verbose_path).unwrap();
    let type_verbose = fs::read_to_string(type_verbose_path).unwrap();
    assert!(command_v.contains("rubash-disabled-describe-bin"));
    assert!(command_v.trim_end().ends_with("echo"));
    assert!(command_v_verbose.contains("echo is "));
    assert!(command_v_verbose.contains("rubash-disabled-describe-bin"));
    assert_eq!(fs::read_to_string(type_t_path).unwrap(), "file\n");
    assert!(type_verbose.contains("echo is "));
    assert!(type_verbose.contains("rubash-disabled-describe-bin"));
    let _ = fs::remove_dir_all(bin_dir);
    for path in [
        command_v_path,
        command_v_verbose_path,
        type_t_path,
        type_verbose_path,
    ] {
        let _ = fs::remove_file(path);
    }
}

#[test]
fn test_nested_command_describe_uses_shell_state() {
    let builtin_function_path = "target/rubash-builtin-command-v-function-output.txt";
    let builtin_verbose_path = "target/rubash-builtin-command-v-verbose-output.txt";
    let nested_function_path = "target/rubash-nested-command-v-function-output.txt";
    let builtin_alias_path = "target/rubash-builtin-command-v-alias-output.txt";
    for path in [
        builtin_function_path,
        builtin_verbose_path,
        nested_function_path,
        builtin_alias_path,
    ] {
        let _ = fs::remove_file(path);
    }
    let input = format!(
        "function ff {{ echo f; }}; \
         shopt -s expand_aliases; \
         alias aa='echo alias value'; \
         builtin command -v ff > {builtin_function_path}; \
         builtin command -V ff > {builtin_verbose_path}; \
         command command -v ff > {nested_function_path}; \
         builtin command -v aa > {builtin_alias_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(builtin_function_path).unwrap(), "ff\n");
    let verbose = fs::read_to_string(builtin_verbose_path).unwrap();
    assert!(verbose.contains("ff is a function"));
    assert_eq!(fs::read_to_string(nested_function_path).unwrap(), "ff\n");
    assert_eq!(
        fs::read_to_string(builtin_alias_path).unwrap(),
        "alias aa='echo alias value'\n"
    );
    for path in [
        builtin_function_path,
        builtin_verbose_path,
        nested_function_path,
        builtin_alias_path,
    ] {
        let _ = fs::remove_file(path);
    }
}

#[test]
fn test_command_without_p_uses_current_path_for_external_command() {
    let output_path = "target/rubash-command-without-p-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("PATH=target/rubash-no-such-bin command sh -c 'echo bad' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 127);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_type_a_reports_builtin_and_path_matches() {
    let bin_dir = "target/rubash-type-a-bin";
    let echo_path = format!("{bin_dir}/echo");
    let output_path = "target/rubash-type-a-output.txt";
    fs::create_dir_all(bin_dir).unwrap();
    fs::write(&echo_path, "").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!("type -a echo > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("PATH", bin_dir);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("echo is a shell builtin\n"));
    assert!(output.contains("echo is target/rubash-type-a-bin/echo\n"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(echo_path);
    let _ = fs::remove_dir(bin_dir);
}

#[test]
fn test_type_ap_reports_only_path_matches() {
    let bin_dir = "target/rubash-type-ap-bin";
    let echo_path = format!("{bin_dir}/echo");
    let output_path = "target/rubash-type-ap-output.txt";
    fs::create_dir_all(bin_dir).unwrap();
    fs::write(&echo_path, "").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!("type -ap echo > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("PATH", bin_dir);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "target/rubash-type-ap-bin/echo\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(echo_path);
    let _ = fs::remove_dir(bin_dir);
}

#[test]
fn test_type_f_skips_shell_functions() {
    let output_path = "target/rubash-type-f-function-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function f {{ echo hi; }}; type -f f; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_type_f_still_reports_builtins() {
    let output_path = "target/rubash-type-f-builtin-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("type -f echo > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "echo is a shell builtin\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_type_long_type_option_reports_kind() {
    let output_path = "target/rubash-type-long-type-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("type --type echo > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "builtin\n");
    let _ = fs::remove_file(output_path);
}
