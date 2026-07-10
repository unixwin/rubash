use super::super::*;
use std::fs;

#[test]
fn test_type_long_all_option_reports_all_matches() {
    let bin_dir = "target/rubash-type-long-all-bin";
    let echo_path = format!("{bin_dir}/echo");
    let output_path = "target/rubash-type-long-all-output.txt";
    fs::create_dir_all(bin_dir).unwrap();
    fs::write(&echo_path, "").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!("type -all echo > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("PATH", bin_dir);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("echo is a shell builtin\n"));
    assert!(output.contains("echo is target/rubash-type-long-all-bin/echo\n"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(echo_path);
    let _ = fs::remove_dir(bin_dir);
}

#[test]
fn test_trap_p_redirects_saved_exit_trap() {
    let output_path = "target/rubash-trap-p-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("trap 'echo bye' EXIT; trap -p EXIT > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "trap -- 'echo bye' EXIT\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_trap_reset_removes_saved_trap() {
    let output_path = "target/rubash-trap-reset-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("trap 'echo bye' EXIT; trap - EXIT; trap -p EXIT > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_trap_ignore_appends_saved_signal_trap() {
    let output_path = "target/rubash-trap-ignore-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("trap '' INT; trap -p INT >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\ntrap -- '' SIGINT\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_trap_accepts_common_signal_names() {
    let output_path = "target/rubash-trap-common-signals-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "trap 'echo pipe' PIPE; trap 'echo alarm' 14; trap -p SIGPIPE ALRM > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "trap -- 'echo pipe' SIGPIPE\ntrap -- 'echo alarm' SIGALRM\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_trap_accepts_realtime_signal_names() {
    let output_path = "target/rubash-trap-realtime-signals-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("trap 'echo rt' RTMIN+1; trap -p SIGRTMIN+1 > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "trap -- 'echo rt' SIGRTMIN+1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exit_runs_exit_trap_and_preserves_status() {
    let output_path = "target/rubash-exit-trap-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("trap 'echo bye > {output_path}' EXIT; exit 7");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(7))));
    assert_eq!(executor.last_exit_code(), 7);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "bye\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exit_trap_exit_overrides_status() {
    let input = "trap 'exit 3' EXIT; exit 7";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(3))));
    assert_eq!(executor.last_exit_code(), 3);
}

#[test]
fn test_normal_completion_runs_exit_trap() {
    let output_path = "target/rubash-normal-exit-trap-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("trap 'echo done > {output_path}' EXIT; true");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);
    let status = executor.run_exit_trap();

    assert!(result.is_ok());
    assert!(status.is_ok());
    assert_eq!(status.unwrap(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "done\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exit_trap_sees_last_status_on_normal_completion() {
    let output_path = "target/rubash-normal-exit-trap-status-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("trap 'echo $? > {output_path}' EXIT; false");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);
    let status = executor.run_exit_trap();

    assert!(result.is_ok());
    assert!(status.is_ok());
    assert_eq!(status.unwrap(), 1);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_trap_invalid_signal_returns_failure() {
    let output_path = "target/rubash-trap-invalid-signal-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("trap 'echo bad' NO_SUCH_SIGNAL; echo $? > {output_path}");
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
fn test_trap_redirects_stderr() {
    let error_path = "target/rubash-trap-stderr-output.txt";
    let status_path = "target/rubash-trap-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("trap 'echo bad' NO_SUCH_SIGNAL 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("trap: NO_SUCH_SIGNAL: invalid signal specification"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_trap_appends_stderr() {
    let error_path = "target/rubash-trap-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("trap 'echo bad' NO_SUCH_SIGNAL 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("trap: NO_SUCH_SIGNAL: invalid signal specification"));
    let _ = fs::remove_file(error_path);
}
