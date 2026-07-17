use super::super::*;
use std::fs;

#[test]
fn test_wait_for_last_background_pid_returns_child_status() {
    let status_path = "target/rubash-wait-last-background-status.txt";
    let error_path = "target/rubash-wait-last-background-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!("false & wait $! 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert_eq!(fs::read_to_string(error_path).unwrap(), "");
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_wait_for_background_jobspec_returns_child_status_and_removes_job() {
    let output_path = "target/rubash-wait-jobspec-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "false & wait %1; printf 'wait:%s\\n' \"$?\" > {output_path}; jobs >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "wait:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_wait_for_current_jobspec_uses_last_background_job() {
    let output_path = "target/rubash-wait-current-jobspec-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "true & wait %+; printf 'wait:%s\\n' \"$?\" > {output_path}; jobs >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "wait:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_wait_n_waits_one_background_job() {
    let output_path = "target/rubash-wait-n-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "false & wait -n; printf 'wait:%s\\n' \"$?\" > {output_path}; jobs >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "wait:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_wait_n_accepts_requested_jobspec() {
    let output_path = "target/rubash-wait-n-jobspec-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "true & wait -n %1; printf 'wait:%s\\n' \"$?\" > {output_path}; jobs >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "wait:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_wait_n_p_assigns_completed_pid() {
    let output_path = "target/rubash-wait-n-p-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "true & launched=$!; wait -n -p done_pid; printf 'wait:%s pid:%s launched:%s\\n' \"$?\" \"$done_pid\" \"$launched\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let rest = output.trim_end().strip_prefix("wait:0 pid:").unwrap();
    let (pid, launched) = rest.split_once(" launched:").unwrap();
    assert!(pid.parse::<u32>().is_ok());
    assert_eq!(pid, launched);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_wait_np_assigns_requested_jobspec_pid() {
    let output_path = "target/rubash-wait-np-jobspec-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "true & launched=$!; wait -np done_pid %1; printf 'wait:%s pid:%s launched:%s\\n' \"$?\" \"$done_pid\" \"$launched\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let rest = output.trim_end().strip_prefix("wait:0 pid:").unwrap();
    let (pid, launched) = rest.split_once(" launched:").unwrap();
    assert!(pid.parse::<u32>().is_ok());
    assert_eq!(pid, launched);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_kill_background_jobspec_removes_job() {
    let output_path = "target/rubash-kill-jobspec-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "while true; do true; done & kill %1; printf 'kill:%s\\n' \"$?\" > {output_path}; jobs >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "kill:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_kill_current_jobspec_uses_last_background_job() {
    let output_path = "target/rubash-kill-current-jobspec-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "while true; do true; done & kill %+; printf 'kill:%s\\n' \"$?\" > {output_path}; jobs >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "kill:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_kill_unknown_jobspec_reports_failure() {
    let error_path = "target/rubash-kill-unknown-jobspec-error.txt";
    let status_path = "target/rubash-kill-unknown-jobspec-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("kill %1 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("kill: %1: no such job"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_wait_for_unknown_pid_returns_notfound() {
    let error_path = "target/rubash-wait-pid-error.txt";
    let status_path = "target/rubash-wait-pid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("wait 999999 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "127\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("wait: pid 999999 is not a child of this shell"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_wait_invalid_operand_returns_failure() {
    let error_path = "target/rubash-wait-invalid-error.txt";
    let status_path = "target/rubash-wait-invalid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("wait abc 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("wait: `abc': not a pid or valid job spec"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_wait_invalid_option_returns_usage() {
    let error_path = "target/rubash-wait-invalid-option-error.txt";
    let status_path = "target/rubash-wait-invalid-option-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("wait -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("wait: -x: invalid option"));
    assert!(error.contains("wait: usage: wait [-fn] [-p var] [id ...]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_fg_without_job_control_returns_failure() {
    let error_path = "target/rubash-fg-error.txt";
    let status_path = "target/rubash-fg-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("fg 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("fg: no job control"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_bg_without_job_control_returns_failure() {
    let error_path = "target/rubash-bg-error.txt";
    let status_path = "target/rubash-bg-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("bg 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("bg: no job control"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_fg_background_pid_waits_and_removes_job() {
    let output_path = "target/rubash-fg-pid-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "false & pid=$!; fg \"$pid\"; printf 'fg:%s\\n' \"$?\" > {output_path}; jobs >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "fg:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bg_background_pid_succeeds_and_keeps_job() {
    let output_path = "target/rubash-bg-pid-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("true & pid=$!; bg \"$pid\"; echo bg:$? > {output_path}; jobs >> {output_path}; disown \"$pid\"");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "bg:0");
    assert_eq!(lines.len(), 2);
    assert!(lines[1].contains("true &"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_suspend_without_job_control_returns_failure() {
    let error_path = "target/rubash-suspend-error.txt";
    let status_path = "target/rubash-suspend-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("suspend 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("suspend: cannot suspend: no job control"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_suspend_invalid_option_returns_usage() {
    let error_path = "target/rubash-suspend-invalid-error.txt";
    let status_path = "target/rubash-suspend-invalid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("suspend -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("suspend: -x: invalid option"));
    assert!(error.contains("suspend: usage: suspend [-f]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_history_without_entries_returns_success() {
    let output_path = "target/rubash-history-empty-output.txt";
    let status_path = "target/rubash-history-empty-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!("history > {output_path}; echo $? > {status_path}");
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
fn test_history_clear_returns_success() {
    let status_path = "target/rubash-history-clear-status.txt";
    let _ = fs::remove_file(status_path);
    let input = format!("history -c; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_history_invalid_option_returns_usage() {
    let error_path = "target/rubash-history-invalid-error.txt";
    let status_path = "target/rubash-history-invalid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("history -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("history: -x: invalid option"));
    assert!(error.contains("history: usage: history [-c] [-d offset]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}
