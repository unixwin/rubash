use super::super::*;
use std::fs;

#[test]
fn test_jobs_without_jobs_returns_success() {
    let output_path = "target/rubash-jobs-empty-output.txt";
    let status_path = "target/rubash-jobs-empty-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("jobs > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_jobs_invalid_option_returns_usage() {
    let error_path = "target/rubash-jobs-invalid-error.txt";
    let status_path = "target/rubash-jobs-invalid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("jobs -z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("jobs: -z: invalid option"));
    assert!(error.contains("jobs: usage: jobs [-lnprs]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_jobs_x_executes_command() {
    let output_path = "target/rubash-jobs-x-output.txt";
    let status_path = "target/rubash-jobs-x-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("jobs -x echo hi there > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hi there\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_jobs_x_uses_command_status() {
    let status_path = "target/rubash-jobs-x-status-output.txt";
    let _ = fs::remove_file(status_path);
    let input = format!("jobs -x false; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_jobs_lists_background_command() {
    let output_path = "target/rubash-jobs-list-output.txt";
    let status_path = "target/rubash-jobs-list-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("true & pid=$!; jobs > {output_path}; wait \"$pid\"; jobs >> {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("Running"));
    assert!(lines[0].ends_with("true &"));
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_jobs_p_lists_background_pid_only() {
    let output_path = "target/rubash-jobs-p-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("true & pid=$!; jobs -p > {output_path}; wait \"$pid\"");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let pid = output.trim_end().parse::<u32>().expect("jobs -p pid");
    assert!(pid > 0);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_disown_without_jobs_reports_current_job_failure() {
    let error_path = "target/rubash-disown-empty-error.txt";
    let status_path = "target/rubash-disown-empty-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("disown 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("disown: current: no such job"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_disown_all_or_running_without_jobs_succeeds() {
    let status_path = "target/rubash-disown-all-status.txt";
    let _ = fs::remove_file(status_path);
    let input = format!("disown -a; echo $? > {status_path}; disown -r; echo $? >> {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n0\n");
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_disown_current_background_job_removes_it_from_jobs() {
    let output_path = "target/rubash-disown-current-output.txt";
    let status_path = "target/rubash-disown-current-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("true & disown; echo $? > {status_path}; jobs > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_disown_background_pid_removes_only_that_job() {
    let output_path = "target/rubash-disown-pid-output.txt";
    let status_path = "target/rubash-disown-pid-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input =
        format!("true & first=$!; false & second=$!; disown \"$first\"; echo $? > {status_path}; jobs > {output_path}; disown \"$second\"");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let output = fs::read_to_string(output_path).unwrap();
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("false &"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_disown_all_background_jobs_clears_jobs() {
    let output_path = "target/rubash-disown-all-output.txt";
    let status_path = "target/rubash-disown-all-jobs-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("true & false & disown -a; echo $? > {status_path}; jobs > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_disown_invalid_option_returns_usage() {
    let error_path = "target/rubash-disown-invalid-error.txt";
    let status_path = "target/rubash-disown-invalid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("disown -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("disown: -x: invalid option"));
    assert!(error.contains("disown: usage: disown [-h] [-ar]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_builtin_and_command_disown_use_shell_builtin() {
    let status_path = "target/rubash-disown-builtin-status.txt";
    let _ = fs::remove_file(status_path);
    let input = format!(
        "builtin disown -a; echo builtin:$? > {status_path}; \
         command disown -a; echo command:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "builtin:0\ncommand:0\n"
    );
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_logout_non_login_shell_reports_error_and_continues() {
    let error_path = "target/rubash-logout-error.txt";
    let output_path = "target/rubash-logout-output.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(output_path);
    let input = format!(
        "logout 2> {error_path}; echo status:$? > {output_path}; echo after >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "status:1\nafter\n"
    );
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("logout: not login shell: use `exit'"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_and_command_logout_use_shell_builtin() {
    let error_path = "target/rubash-logout-builtin-error.txt";
    let output_path = "target/rubash-logout-builtin-output.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(output_path);
    let input = format!(
        "builtin logout 2> {error_path}; echo builtin:$? > {output_path}; \
         command logout 2>> {error_path}; echo command:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "builtin:1\ncommand:1\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert_eq!(
        error.matches("logout: not login shell: use `exit'").count(),
        2
    );
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_wait_without_operands_returns_success() {
    let status_path = "target/rubash-wait-empty-status.txt";
    let _ = fs::remove_file(status_path);
    let input = format!("wait; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(status_path);
}
