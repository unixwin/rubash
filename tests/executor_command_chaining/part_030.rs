use super::super::*;
use std::fs;

#[test]
fn test_command_uses_external_test_when_builtin_is_disabled() {
    let bin_dir = "target/rubash-disabled-command-test-bin";
    let script_path = format!("{bin_dir}/test");
    let output_path = "target/rubash-disabled-command-test-output.txt";
    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    write_executable(&script_path, "echo external-command-test\n").unwrap();
    let input = format!("enable -n test; command test > {output_path}; enable test");
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
        "external-command-test\n"
    );
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir(bin_dir);
}

#[test]
fn test_disabled_read_builtins_use_external_commands() {
    let bin_dir = "target/rubash-disabled-read-bin";
    let read_path = format!("{bin_dir}/read");
    let mapfile_path = format!("{bin_dir}/mapfile");
    let readarray_path = format!("{bin_dir}/readarray");
    let read_output_path = "target/rubash-disabled-read-output.txt";
    let command_read_output_path = "target/rubash-disabled-command-read-output.txt";
    let mapfile_output_path = "target/rubash-disabled-mapfile-output.txt";
    let readarray_output_path = "target/rubash-disabled-readarray-output.txt";
    let _ = fs::remove_dir_all(bin_dir);
    for path in [
        read_output_path,
        command_read_output_path,
        mapfile_output_path,
        readarray_output_path,
    ] {
        let _ = fs::remove_file(path);
    }
    fs::create_dir_all(bin_dir).unwrap();
    write_executable(&read_path, "echo external-read\n").unwrap();
    write_executable(&mapfile_path, "echo external-mapfile\n").unwrap();
    write_executable(&readarray_path, "echo external-readarray\n").unwrap();
    let input = format!(
        "enable -n read mapfile readarray; \
         read value <<< hi > {read_output_path}; \
         command read value <<< hi > {command_read_output_path}; \
         mapfile arr <<< hi > {mapfile_output_path}; \
         readarray arr <<< hi > {readarray_output_path}"
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
    assert_eq!(
        fs::read_to_string(read_output_path).unwrap(),
        "external-read\n"
    );
    assert_eq!(
        fs::read_to_string(command_read_output_path).unwrap(),
        "external-read\n"
    );
    assert_eq!(
        fs::read_to_string(mapfile_output_path).unwrap(),
        "external-mapfile\n"
    );
    assert_eq!(
        fs::read_to_string(readarray_output_path).unwrap(),
        "external-readarray\n"
    );
    let _ = fs::remove_dir_all(bin_dir);
    for path in [
        read_output_path,
        command_read_output_path,
        mapfile_output_path,
        readarray_output_path,
    ] {
        let _ = fs::remove_file(path);
    }
}

#[test]
fn test_disabled_echo_builtin_uses_external_command() {
    let bin_dir = "target/rubash-disabled-echo-bin";
    let script_path = format!("{bin_dir}/echo");
    let output_path = "target/rubash-disabled-echo-output.txt";
    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    write_executable(&script_path, "printf 'external-echo %s\\n' \"$*\"\n").unwrap();
    let input = format!(
        "enable -n echo; echo hello > {output_path}; enable echo; echo builtin >> {output_path}"
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
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "external-echo hello\nbuiltin\n"
    );
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir(bin_dir);
}

#[test]
fn test_command_uses_external_echo_when_builtin_is_disabled() {
    let bin_dir = "target/rubash-disabled-command-echo-bin";
    let script_path = format!("{bin_dir}/echo");
    let output_path = "target/rubash-disabled-command-echo-output.txt";
    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    write_executable(
        &script_path,
        "printf 'external-command-echo %s\\n' \"$*\"\n",
    )
    .unwrap();
    let input = format!("enable -n echo; command echo hello > {output_path}; enable echo");
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
        "external-command-echo hello\n"
    );
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir(bin_dir);
}

#[test]
fn test_disabled_printf_builtin_uses_external_command() {
    let bin_dir = "target/rubash-disabled-printf-bin";
    let script_path = format!("{bin_dir}/printf");
    let output_path = "target/rubash-disabled-printf-output.txt";
    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    write_executable(&script_path, "echo external-printf \"$@\"\n").unwrap();
    let input = format!(
        "enable -n printf; printf hello > {output_path}; enable printf; printf '%s\\n' builtin >> {output_path}"
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
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "external-printf hello\nbuiltin\n"
    );
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir(bin_dir);
}

#[test]
fn test_command_uses_external_printf_when_builtin_is_disabled() {
    let bin_dir = "target/rubash-disabled-command-printf-bin";
    let script_path = format!("{bin_dir}/printf");
    let output_path = "target/rubash-disabled-command-printf-output.txt";
    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    write_executable(&script_path, "echo external-command-printf \"$@\"\n").unwrap();
    let input = format!("enable -n printf; command printf hello > {output_path}; enable printf");
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
        "external-command-printf hello\n"
    );
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir(bin_dir);
}
