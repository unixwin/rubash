use super::super::*;
use std::fs;

#[test]
fn test_printf_time_format_minus_two_uses_shell_start_time() {
    let output_path = "target/rubash-printf-time-shell-start-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("TZ=UTC printf '%(%s)T' -2 > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    let shell_start = executor
        .get_env("__RUBASH_SHELL_START_EPOCH")
        .unwrap()
        .to_string();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), shell_start);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_printf_invalid_time_format_warns_and_outputs_raw_format() {
    let output_path = "target/rubash-printf-invalid-time-output.txt";
    let status_path = "target/rubash-printf-invalid-time-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("printf '%(abde)Z\\n' -1 > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "%(abde)Z\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_epochseconds_expands_to_current_epoch_time() {
    let output_path = "target/rubash-epochseconds-output.txt";
    let _ = fs::remove_file(output_path);
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let input = format!("printf '%s\\n' \"$EPOCHSECONDS\" \"${{EPOCHSECONDS}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let values = fs::read_to_string(output_path)
        .unwrap()
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 2);
    assert!(values.iter().all(|value| (before..=after).contains(value)));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_epochrealtime_expands_with_microseconds() {
    let output_path = "target/rubash-epochrealtime-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '%s\\n' \"$EPOCHREALTIME\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let value = output.trim_end();
    let (seconds, micros) = value.split_once('.').expect("epoch realtime decimal");
    assert!(seconds.parse::<i64>().is_ok());
    assert_eq!(micros.len(), 6);
    assert!(micros.chars().all(|ch| ch.is_ascii_digit()));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_seconds_assignment_resets_dynamic_counter() {
    let output_path = "target/rubash-seconds-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("SECONDS=7; printf '%s\\n' \"$SECONDS\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let seconds: i64 = fs::read_to_string(output_path)
        .unwrap()
        .trim_end()
        .parse()
        .unwrap();
    assert!((7..=8).contains(&seconds));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_argv0_assignment_updates_zero_parameter() {
    let output_path = "target/rubash-bash-argv0-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("BASH_ARGV0=hello; printf '%s:%s\\n' \"$0\" \"$BASH_ARGV0\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hello:hello\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_argv0_defaults_to_shell_name() {
    let output_path = "target/rubash-bash-argv0-default-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '%s:%s\\n' \"$0\" \"$BASH_ARGV0\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "rubash:rubash\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_argv0_reflects_script_name() {
    let output_path = "target/rubash-bash-argv0-script-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '%s:%s\\n' \"$0\" \"$BASH_ARGV0\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("__RUBASH_SCRIPT_NAME", "./script.sh");

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "./script.sh:./script.sh\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_zero_parameter_supports_pattern_removal() {
    let output_path = "target/rubash-zero-param-pattern-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '%s\\n' \"${{0##*/}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("__RUBASH_SCRIPT_NAME", "./bin/script.sh");

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "script.sh\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_dynamic_bash_parameter_lengths_use_current_values() {
    let output_path = "target/rubash-dynamic-param-length-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("printf '%s:%s:%s\\n' \"${{#BASH_ARGV0}}\" \"${{#BASHPID}}\" \"${{#BASH_SUBSHELL}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("__RUBASH_SCRIPT_NAME", "./script.sh");

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("11:{}:1\n", std::process::id().to_string().chars().count())
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_argv0_assignment_inside_function_updates_zero_parameter() {
    let output_path = "target/rubash-bash-argv0-function-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "setarg0() {{ BASH_ARGV0=\"$1\"; }}; setarg0 arg0; printf '%s:%s\\n' \"$0\" \"$BASH_ARGV0\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "arg0:arg0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_random_expands_to_15_bit_values_and_advances() {
    let output_path = "target/rubash-random-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '%s\\n' \"$RANDOM\" \"$RANDOM\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let values = fs::read_to_string(output_path)
        .unwrap()
        .lines()
        .map(|line| line.parse::<u32>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 2);
    assert!(values.iter().all(|value| *value <= 32767));
    assert_ne!(values[0], values[1]);
    let _ = fs::remove_file(output_path);
}
