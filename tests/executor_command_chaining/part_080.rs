use super::super::*;
use std::fs;
use std::thread;
use std::time::Duration;

#[test]
fn test_combined_stdout_stderr_redirect_captures_brace_group() {
    let output_path = "target/rubash-combined-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("{{ echo out; no_such_combined_redirect_cmd; }} &> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 127);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("no_such_combined_redirect_cmd: command not found"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_combined_stdout_stderr_append_captures_brace_group() {
    let output_path = "target/rubash-combined-append-output.txt";
    fs::write(output_path, "first\n").unwrap();
    let input = format!("{{ echo out; no_such_combined_append_cmd; }} &>> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 127);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("first\n"));
    assert!(output.contains("out\n"));
    assert!(output.contains("no_such_combined_append_cmd: command not found"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_combined_output_process_substitution_captures_stdout_and_stderr() {
    let output_path = target_test_path("rubash-combined-process-substitution-output.txt");
    let helper_path = target_test_path(if cfg!(windows) {
        "rubash-combined-process-substitution.cmd"
    } else {
        "rubash-combined-process-substitution.sh"
    });
    let shell_output_path = shell_test_path(&output_path);
    let shell_helper_path = shell_test_path(&helper_path);
    let _ = fs::remove_file(&output_path);
    if cfg!(windows) {
        write_executable(&helper_path, "@echo out\r\n@echo err 1>&2\r\n").unwrap();
    } else {
        write_executable(
            &helper_path,
            "#!/bin/sh\nprintf 'out\\n'\nprintf 'err\\n' >&2\n",
        )
        .unwrap();
    }
    let input = format!("{shell_helper_path} &> >(cat > {shell_output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.contains("out"));
    assert!(output.contains("err"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(helper_path);
}

#[test]
fn test_combined_append_process_substitution_captures_stdout_and_stderr() {
    let output_path = target_test_path("rubash-combined-append-process-substitution-output.txt");
    let helper_path = target_test_path(if cfg!(windows) {
        "rubash-combined-append-process-substitution.cmd"
    } else {
        "rubash-combined-append-process-substitution.sh"
    });
    let shell_output_path = shell_test_path(&output_path);
    let shell_helper_path = shell_test_path(&helper_path);
    let _ = fs::remove_file(&output_path);
    if cfg!(windows) {
        write_executable(&helper_path, "@echo out\r\n@echo err 1>&2\r\n").unwrap();
    } else {
        write_executable(
            &helper_path,
            "#!/bin/sh\nprintf 'out\\n'\nprintf 'err\\n' >&2\n",
        )
        .unwrap();
    }
    let input = format!("{shell_helper_path} &>> >(cat > {shell_output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.contains("out"));
    assert!(output.contains("err"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(helper_path);
}

#[test]
fn test_brace_group_combined_process_substitution_captures_whole_body() {
    let output_path = "target/rubash-brace-combined-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("{{ echo out; read -u x value; }} &> >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("read: x: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_brace_group_combined_append_process_substitution_captures_whole_body() {
    let output_path = "target/rubash-brace-combined-append-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("{{ echo out; read -u x value; }} &>> >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("read: x: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_subshell_combined_process_substitution_captures_whole_body() {
    let output_path = "target/rubash-subshell-combined-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("( echo out; read -u x value ) &> >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("read: x: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_subshell_combined_append_process_substitution_captures_whole_body() {
    let output_path = "target/rubash-subshell-combined-append-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("( echo out; read -u x value ) &>> >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("read: x: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_combined_process_substitution_captures_body() {
    let output_path = "target/rubash-if-combined-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("if true; then echo out; read -u x value; fi &> >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("read: x: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_while_combined_append_process_substitution_captures_body() {
    let output_path = "target/rubash-while-combined-append-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("while true; do echo out; read -u x value; break; done &>> >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("read: x: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_combined_process_substitution_captures_body() {
    let output_path = "target/rubash-for-combined-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("for item in one; do echo out; read -u x value; done &> >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("read: x: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_combined_append_process_substitution_captures_clause_body() {
    let output_path = "target/rubash-case-combined-append-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("case one in one) echo out; read -u x value ;; esac &>> >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("read: x: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_combined_process_substitution_captures_body() {
    let output_path = "target/rubash-select-combined-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "select item in one; do echo out; read -u x value; break; done &> >(cat > {output_path}) <<< 1"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("read: x: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_brace_group_combined_process_substitution_captures_body() {
    let output_path = "target/rubash-time-brace-combined-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time {{ echo out; read -u x value; }} &> >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("read: x: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_stderr_fd_copy_after_stdout_redirect_captures_brace_group() {
    let output_path = "target/rubash-fd-copy-after-stdout-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("{{ echo out; no_such_fd_copy_cmd; }} > {output_path} 2>&1");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 127);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("out\n"));
    assert!(output.contains("no_such_fd_copy_cmd: command not found"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_stderr_fd_copy_after_stdout_append_captures_brace_group() {
    let output_path = "target/rubash-fd-copy-after-stdout-append-output.txt";
    fs::write(output_path, "first\n").unwrap();
    let input = format!("{{ echo out; no_such_fd_copy_append_cmd; }} >> {output_path} 2>&1");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 127);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("first\n"));
    assert!(output.contains("out\n"));
    assert!(output.contains("no_such_fd_copy_append_cmd: command not found"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_stderr_fd_close_does_not_run_dash_command_or_create_file() {
    let output_path = "target/rubash-stderr-fd-close-output.txt";
    let closed_path = "&-";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(closed_path);
    let input = format!("echo out 2>&- > {output_path}; echo status:$? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "out\nstatus:0\n");
    assert!(!std::path::Path::new(closed_path).exists());
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_leading_redirects_apply_to_simple_command() {
    let output_path = "target/rubash-leading-redirect-output.txt";
    let error_path = "target/rubash-leading-redirect-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("> {output_path} echo out; 2> {error_path} sh -c 'printf err >&2'");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "out\n");
    assert_eq!(fs::read_to_string(error_path).unwrap(), "err");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_leading_combined_redirects_apply_to_simple_command() {
    let output_path = "target/rubash-leading-combined-redirect-output.txt";
    let append_path = "target/rubash-leading-combined-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(append_path, "first\n").unwrap();
    let input = format!(
        "&> {output_path} sh -c 'echo out; printf err >&2'; \
         &>> {append_path} sh -c 'echo append-out; printf append-err >&2'"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "out\nerr");
    assert_eq!(
        fs::read_to_string(append_path).unwrap(),
        "first\nappend-out\nappend-err"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(append_path);
}

#[test]
fn test_leading_process_substitution_redirects_apply_to_simple_command() {
    let output_path = "target/rubash-leading-process-substitution-output.txt";
    let combined_path = "target/rubash-leading-combined-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(combined_path);
    let input = format!(
        "> >(cat > {output_path}) echo out; \
         &> >(cat > {combined_path}) sh -c 'echo both-out; printf both-err >&2'"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let mut output = String::new();
    let mut combined = String::new();
    for _ in 0..20 {
        output = fs::read_to_string(output_path).unwrap_or_default();
        combined = fs::read_to_string(combined_path).unwrap_or_default();
        if output == "out\n" && combined == "both-out\nboth-err" {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(output, "out\n");
    assert_eq!(combined, "both-out\nboth-err");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(combined_path);
}

#[test]
fn test_stdout_fd_close_does_not_create_file() {
    let output_path = "target/rubash-stdout-fd-close-output.txt";
    let closed_path = "&-";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(closed_path);
    let input = format!("echo hidden 1>&-; echo shown > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "shown\n");
    assert!(!std::path::Path::new(closed_path).exists());
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_write_redirect_feeds_external_stdin() {
    let input_path = "target/rubash-read-write-redirect-input.txt";
    let output_path = "target/rubash-read-write-redirect-output.txt";
    fs::write(input_path, "alpha\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!("cat <> {input_path} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_write_redirect_on_colon_creates_file() {
    let input_path = "target/rubash-read-write-redirect-create.txt";
    let output_path = "target/rubash-read-write-redirect-create-output.txt";
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
    let input = format!(": <> {input_path}; test -f {input_path}; echo status:$? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "status:0\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_write_redirect_fd_prefix_feeds_read_u() {
    let input_path = "target/rubash-read-write-fd-prefix-input.txt";
    let output_path = "target/rubash-read-write-fd-prefix-output.txt";
    fs::write(input_path, "prefixed\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!("read -u 3 value 3<>{input_path}; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "prefixed\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_stdin_redirect_fd_prefix_without_space_feeds_external_stdin() {
    let input_path = "target/rubash-stdin-fd-prefix-input.txt";
    let output_path = "target/rubash-stdin-fd-prefix-output.txt";
    fs::write(input_path, "prefixed\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!("cat 0<{input_path} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "prefixed\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_u_uses_numbered_input_redirect_fd() {
    let input_path = "target/rubash-read-u-numbered-fd-input.txt";
    let output_path = "target/rubash-read-u-numbered-fd-output.txt";
    fs::write(input_path, "fd-line\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!("read -u 3 value 3<{input_path}; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "fd-line\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_u_rejects_invalid_fd_specifications() {
    let output_path = "target/rubash-read-u-invalid-fd-status.txt";
    let error_path = "target/rubash-read-u-invalid-fd-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "read -u x value <<< abc 2> {error_path}; echo word:$? > {output_path}; \
         read -u-1 value <<< abc 2>> {error_path}; echo compact_negative:$? >> {output_path}; \
         read -u2147483648 value <<< abc 2>> {error_path}; echo too_large:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "word:1\ncompact_negative:1\ntoo_large:1\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("read: x: invalid file descriptor specification"));
    assert!(error.contains("read: -1: invalid file descriptor specification"));
    assert!(error.contains("read: 2147483648: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_read_u_missing_argument_reports_usage() {
    let output_path = "target/rubash-read-u-missing-status.txt";
    let error_path = "target/rubash-read-u-missing-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("read -u 2> {error_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("read: -u: option requires an argument"));
    assert!(error.contains("read: usage:"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_read_u_reports_bad_fd_for_unopened_or_closed_fd() {
    let output_path = "target/rubash-read-u-bad-fd-status.txt";
    let error_path = "target/rubash-read-u-bad-fd-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "read -u3 value 2> {error_path}; echo unopened:$? > {output_path}; \
         read -u 3 value 3<&- 2>> {error_path}; echo closed:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "unopened:1\nclosed:1\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert_eq!(
        error
            .matches("read: 3: invalid file descriptor: Bad file descriptor")
            .count(),
        2
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_read_u_allows_open_fd_at_eof() {
    let output_path = "target/rubash-read-u-open-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("read -u3 value 3<<< ''; printf '%s:<%s>' \"$?\" \"$value\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0:<>");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_input_fd_copy_reads_virtual_fd() {
    let output_path = "target/rubash-input-fd-copy-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read first <&3 3<<EOF; echo $first > {output_path}\nfrom-fd\nEOF");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "from-fd\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_dynamic_output_fd_writes_through_named_fd() {
    let output_path = "target/rubash-dynamic-output-fd.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "exec {{fd}}>{output_path}; echo alpha >&$fd; printf '%s\\n' beta >&$fd; exec {{fd}}>&-"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_dynamic_output_fd_copies_persistent_stdout_redirect() {
    let output_path = "target/rubash-dynamic-output-fd-copy-stdout.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("exec > {output_path}; exec {{fd}}>&1; echo copied >&$fd; exec {{fd}}>&-");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "copied\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_dynamic_input_fd_reads_through_named_fd() {
    let input_path = "target/rubash-dynamic-input-fd.txt";
    let output_path = "target/rubash-dynamic-input-fd-output.txt";
    let error_path = "target/rubash-dynamic-input-fd-error.txt";
    fs::write(input_path, "alpha\nbeta\n").unwrap();
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "exec {{fd}}<{input_path}; read -u $fd first; read -u $fd second; \
         exec {{fd}}<&-; read -u $fd closed 2> {error_path}; \
         echo \"$first/$second/$closed:$?\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha/beta/:1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("invalid file descriptor specification"));
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_exec_numeric_input_fd_reads_through_static_fd() {
    let input_path = "target/rubash-static-input-fd.txt";
    let output_path = "target/rubash-static-input-fd-output.txt";
    fs::write(input_path, "alpha\nbeta\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!("exec 3<{input_path}; read -u 3 first; read -u 3 second; echo $first/$second > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha/beta\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_dynamic_input_fd_copies_persistent_stdin_redirect() {
    let input_path = "target/rubash-dynamic-input-fd-copy-stdin.txt";
    let output_path = "target/rubash-dynamic-input-fd-copy-stdin-output.txt";
    fs::write(input_path, "alpha\nbeta\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!(
        "exec < {input_path}; read first; exec {{fd}}<&0; read -u $fd second; \
         exec {{fd}}<&-; echo $first/$second > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha/beta\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_external_command_reads_dynamic_input_fd() {
    let input_path = "target/rubash-external-dynamic-input-fd.txt";
    let output_path = "target/rubash-external-dynamic-input-fd-output.txt";
    let status_path = "target/rubash-external-dynamic-input-fd-status.txt";
    fs::write(input_path, "alpha\nbeta\n").unwrap();
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "exec {{fd}}<{input_path}; cat <&$fd > {output_path}; \
         read -u $fd after; echo \"$?:$after\" > {status_path}; exec {{fd}}<&-"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1:\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_external_command_writes_dynamic_output_fd() {
    let input_path = "target/rubash-external-dynamic-output-fd-input.txt";
    let output_path = "target/rubash-external-dynamic-output-fd.txt";
    fs::write(input_path, "from-cat\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!(
        "exec {{fd}}>{output_path}; echo before >&$fd; cat {input_path} >&$fd; exec {{fd}}>&-"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("before\n"));
    assert!(output.contains("from-cat\n"));
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_dynamic_read_write_fd_reads_and_writes() {
    let data_path = "target/rubash-dynamic-read-write-fd.txt";
    let output_path = "target/rubash-dynamic-read-write-fd-output.txt";
    fs::write(data_path, "alpha\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!(
        "exec {{fd}}<>{data_path}; read -u $fd first; echo beta >&$fd; \
         exec {{fd}}>&-; echo $first > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    assert_eq!(fs::read_to_string(data_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(data_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_dynamic_fd_here_string_persists_for_external_command() {
    let output_path = "target/rubash-dynamic-fd-here-string-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("exec {{fd}}<<<alpha; cat <&$fd > {output_path}; exec {{fd}}<&-");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_dynamic_fd_heredoc_persists_for_external_command() {
    let output_path = "target/rubash-dynamic-fd-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("exec {{fd}}<<EOF\nalpha\nbeta\nEOF\ncat <&$fd > {output_path}; exec {{fd}}<&-");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_fd_process_substitution_persists_for_external_command() {
    let output_path = "target/rubash-exec-fd-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("exec 3< <(printf 'alpha\\nbeta\\n'); cat <&3 > {output_path}; exec 3<&-");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_dynamic_fd_process_substitution_persists_for_external_command() {
    let output_path = "target/rubash-dynamic-fd-process-substitution-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "exec {{fd}}< <(printf 'alpha\\nbeta\\n'); cat <&$fd > {output_path}; exec {{fd}}<&-"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_fd_output_process_substitution_runs_on_close() {
    let output_path = "target/rubash-exec-fd-output-process-substitution.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "exec 3> >(cat > {output_path}); printf '%s\\n' alpha >&3; printf '%s\\n' beta >&3; exec 3>&-"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_dynamic_fd_output_process_substitution_runs_on_close() {
    let output_path = "target/rubash-dynamic-fd-output-process-substitution.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "exec {{fd}}> >(cat > {output_path}); printf '%s\\n' alpha >&$fd; printf '%s\\n' beta >&$fd; exec {{fd}}>&-"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_stderr_process_substitution_runs_on_close() {
    let output_path = "target/rubash-stderr-process-substitution.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "exec 2> >(cat > {output_path}); printf '%s\\n' alpha >&2; printf '%s\\n' beta >&2; exec 2>&-"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_append_output_process_substitution_feeds_command_stdin() {
    let output_path = "target/rubash-append-output-process-substitution.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("echo appended >> >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "appended\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_fd_append_process_substitution_runs_on_close() {
    let output_path = "target/rubash-exec-fd-append-process-substitution.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "exec 3>> >(cat > {output_path}); printf '%s\\n' alpha >&3; printf '%s\\n' beta >&3; exec 3>&-"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_dynamic_fd_append_process_substitution_runs_on_close() {
    let output_path = "target/rubash-dynamic-fd-append-process-substitution.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "exec {{fd}}>> >(cat > {output_path}); printf '%s\\n' alpha >&$fd; printf '%s\\n' beta >&$fd; exec {{fd}}>&-"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_stderr_append_process_substitution_runs_on_close() {
    let output_path = "target/rubash-stderr-append-process-substitution.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "exec 2>> >(cat > {output_path}); printf '%s\\n' alpha >&2; printf '%s\\n' beta >&2; exec 2>&-"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_input_fd_close_makes_read_fail_without_hanging() {
    let output_path = "target/rubash-input-fd-close-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read value <&-; echo status:$?:$value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "status:1:\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_u_reads_fd_here_string() {
    let output_path = "target/rubash-read-u-fd-here-string-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -u 3 value 3<<<alpha; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_external_command_reads_fd_here_string() {
    let output_path = "target/rubash-external-fd-here-string-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat <&3 3<<<alpha > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_fd_here_string_persists_for_external_command() {
    let output_path = "target/rubash-exec-fd-here-string-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("exec 3<<<alpha; cat <&3 > {output_path}; exec 3<&-");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_exec_fd_heredoc_persists_for_external_command() {
    let output_path = "target/rubash-exec-fd-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("exec 3<<EOF\nalpha\nbeta\nEOF\ncat <&3 > {output_path}; exec 3<&-");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pipeline_feeds_while_command_stage() {
    let output_path = "target/rubash-pipeline-while-stage-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("printf 'a\\nb\\n' | while read value; do echo \"<$value>\"; done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<a>\n<b>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pipe_stderr_operator_feeds_next_stage() {
    let output_path = "target/rubash-pipe-stderr-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '%(bad)s\\n' |& grep warning > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("invalid time format specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_command_can_be_pipeline_stage() {
    let output_path = "target/rubash-time-pipeline-stage-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf 'alpha\\n' | time cat > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
    assert!(pipeline.stages[1].time_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_inverted_time_command_pipeline_stage_flips_status() {
    let output_path = "target/rubash-inverted-time-pipeline-stage-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "printf 'alpha\\n' | time ! grep beta > {output_path}; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
    assert!(pipeline.stages[1].time_command.as_ref().unwrap().inverted);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "status:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pipeline_feeds_brace_group_stage() {
    let output_path = "target/rubash-pipeline-brace-stage-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "printf 'first\\nsecond\\n' | {{ read first; read second; echo \"$second/$first\"; }} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "second/first\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pipe_stderr_operator_feeds_brace_group_stage_stderr() {
    let output_path = "target/rubash-pipe-stderr-brace-stage-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("{{ builtin nosuch; }} |& grep 'not a shell builtin' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("builtin: nosuch: not a shell builtin"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_command_pipeline_stage_feeds_next_command() {
    let output_path = "target/rubash-pipeline-for-stage-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("for value in a b; do echo $value; done | wc -l > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_command_pipeline_stage_feeds_next_command() {
    let output_path = "target/rubash-pipeline-if-stage-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("if true; then echo yes; fi | grep yes > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_command_pipeline_stage_feeds_next_command() {
    let output_path = "target/rubash-pipeline-case-stage-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("case yes in yes) echo yes ;; *) echo no ;; esac | grep yes > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].pipeline_command.is_some());
    assert!(ast.commands[0].pipeline_command.as_ref().unwrap().stages[0]
        .case_command
        .is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_grouping_and_select_command_pipeline_stages_feed_next_command() {
    let output_path = "target/rubash-pipeline-grouping-select-stage-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "( echo subshell ) | cat > {output_path}; \
         select value in one two; do echo select:$value; break; done <<< 2 | cat >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].pipeline_command.as_ref().unwrap().stages[0]
        .subshell_command
        .is_some());
    assert!(ast.commands[1].pipeline_command.as_ref().unwrap().stages[0]
        .select_command
        .is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "subshell\nselect:two\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_inverted_compound_commands_flip_status() {
    let output_path = "target/rubash-inverted-compound-status.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "! for value in one; do false; done; echo for:$? > {output_path}; \
         ! while false; do :; done; echo while:$? >> {output_path}; \
         ! if false; then :; fi; echo if:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "for:0\nwhile:1\nif:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_inverted_case_and_test_commands_flip_status() {
    let output_path = "target/rubash-inverted-case-test-status.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "! case yes in yes) true ;; esac; echo case:$? > {output_path}; \
         ! (( 0 )); echo arith:$? >> {output_path}; \
         ! [[ no == yes ]]; echo cond:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0]
        .inverted_command
        .as_ref()
        .unwrap()
        .command
        .case_command
        .is_some());
    assert!(ast.commands[2]
        .inverted_command
        .as_ref()
        .unwrap()
        .command
        .arithmetic_command
        .is_some());
    assert!(ast.commands[4]
        .inverted_command
        .as_ref()
        .unwrap()
        .command
        .conditional_command
        .is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "case:1\narith:0\ncond:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_inverted_grouping_and_select_commands_flip_status() {
    let output_path = "target/rubash-inverted-grouping-select-status.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "! {{ true; }}; echo brace:$? > {output_path}; \
         ! ( false ); echo subshell:$? >> {output_path}; \
         ! select value in one two; do true; break; done <<< 2; echo select:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0]
        .inverted_command
        .as_ref()
        .unwrap()
        .command
        .brace_group
        .is_some());
    assert!(ast.commands[2]
        .inverted_command
        .as_ref()
        .unwrap()
        .command
        .subshell_command
        .is_some());
    assert!(ast.commands[4]
        .inverted_command
        .as_ref()
        .unwrap()
        .command
        .select_command
        .is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "brace:1\nsubshell:0\nselect:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_inverted_function_definitions_still_define_functions() {
    let output_path = "target/rubash-inverted-function-definition-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "! function first {{ echo first > {output_path}; }}; first; \
         ! second() {{ echo second >> {output_path}; }}; second"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(ast.commands[0]
        .inverted_command
        .as_ref()
        .unwrap()
        .command
        .function_command
        .is_some());
    assert!(ast.commands[2]
        .inverted_command
        .as_ref()
        .unwrap()
        .command
        .function_command
        .is_some());
    assert_eq!(fs::read_to_string(output_path).unwrap(), "first\nsecond\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_body_can_be_for_command() {
    let output_path = "target/rubash-function-for-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("f() for x in a b; do echo $x >> {output_path}; done; f");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\nb\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_definition_and_or_connector_executes_rhs() {
    let output_path = "target/rubash-function-and-or-connector-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("f() {{ echo body; }} && echo defined > {output_path}; f >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "defined\nbody\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_body_can_be_arithmetic_command() {
    let output_path = "target/rubash-function-arithmetic-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() (( 1 )); f; echo true:$? > {output_path}; \
         g() (( 0 )); g; echo false:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "true:0\nfalse:1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_body_can_be_case_command() {
    let output_path = "target/rubash-function-case-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() case $1 in a) echo alpha > {output_path} ;; *) echo other > {output_path} ;; esac; f a"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_body_can_contain_select_command() {
    let output_path = "target/rubash-case-nested-select-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "case x in x) select choice in inner; do echo $choice > {output_path}; break; done <<< 1 ;; esac"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let body = &ast.commands[0].case_command.as_ref().unwrap().clauses[0].body;
    assert!(body.iter().any(|command| command.select_command.is_some()));
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "inner\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_subshell_command_keeps_case_pattern_parentheses() {
    let output_path = "target/rubash-subshell-case-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "( case beta in alpha) printf alpha ;; beta) printf beta ;; esac ) > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "beta");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_body_can_be_if_command_sequence() {
    let output_path = "target/rubash-function-if-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("f() if true; then echo yes > {output_path}; else echo no > {output_path}; fi; f");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_body_can_be_while_command_sequence() {
    let output_path = "target/rubash-function-while-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("n=0; f() while [[ $n -lt 2 ]]; do echo $n >> {output_path}; (( n++ )); done; f");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_body_can_be_until_command_sequence() {
    let output_path = "target/rubash-function-until-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("n=0; f() until [[ $n -ge 2 ]]; do echo $n >> {output_path}; (( n++ )); done; f");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_body_can_be_select_command() {
    let output_path = "target/rubash-function-select-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() select choice in inner; do echo $choice > {output_path}; break; done <<< 1; f"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "inner\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_body_can_be_coproc_command() {
    let output_path = "target/rubash-function-coproc-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("f() coproc MYC (( 1 )); f; echo pid:${{MYC_PID:+set}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "pid:set\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_body_can_be_time_command() {
    let output_path = "target/rubash-function-time-body-output.txt";
    let error_path = "target/rubash-function-time-body-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("f() time echo timed > {output_path}; f 2> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(ast.commands[0].function_command.as_ref().unwrap().body[0]
        .time_command
        .is_some());
    assert_eq!(fs::read_to_string(output_path).unwrap(), "timed\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_function_body_can_be_subshell_command() {
    let output_path = "target/rubash-function-subshell-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() ( x=inner; echo in:$x > {output_path} ); x=outer; f; echo out:$x >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "in:inner\nout:outer\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_keyword_can_use_subshell_body_without_signature_parentheses() {
    let output_path = "target/rubash-function-keyword-subshell-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function f ( echo hi ); f > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hi\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_keyword_with_parentheses_can_use_subshell_body() {
    let output_path = "target/rubash-function-keyword-paren-subshell-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function f () ( echo hi ); f > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(
        ast.commands[0]
            .function_command
            .as_ref()
            .unwrap()
            .has_parentheses
    );
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hi\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_subshell_body_keeps_case_pattern_parentheses() {
    let output_path = "target/rubash-function-subshell-case-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() ( case beta in alpha) printf alpha ;; beta) printf beta ;; esac ); f > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "beta");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_body_can_be_conditional_command() {
    let output_path = "target/rubash-function-conditional-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() [[ $1 == a* && $2 -gt 1 ]]; f alpha 2; echo yes:$? > {output_path}; f beta 2; echo no:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes:0\nno:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_named_coproc_executes_for_body() {
    let output_path = "target/rubash-coproc-for-body-output.txt";
    let status_path = "target/rubash-coproc-for-body-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "coproc MYC for x in a b; do echo $x >> {output_path}; done; echo pid:${{MYC_PID:+set}} > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");

    let mut output = String::new();
    for _ in 0..20 {
        if let Ok(contents) = fs::read_to_string(output_path) {
            output = contents;
            if output == "a\nb\n" {
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(output, "a\nb\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_named_coproc_executes_case_body() {
    let output_path = "target/rubash-coproc-case-body-output.txt";
    let status_path = "target/rubash-coproc-case-body-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "coproc MYC case beta in alpha) echo alpha > {output_path} ;; beta) echo beta > {output_path} ;; esac; echo pid:${{MYC_PID:+set}} > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert!(coproc.body.as_ref().unwrap()[0].case_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");

    let mut output = String::new();
    for _ in 0..20 {
        if let Ok(contents) = fs::read_to_string(output_path) {
            output = contents;
            if output == "beta\n" {
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(output, "beta\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_named_coproc_executes_if_body() {
    let output_path = "target/rubash-coproc-if-body-output.txt";
    let status_path = "target/rubash-coproc-if-body-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "coproc MYC if true; then echo yes > {output_path}; else echo no > {output_path}; fi; echo pid:${{MYC_PID:+set}} > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert!(coproc.body.as_ref().unwrap()[0].if_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");

    let mut output = String::new();
    for _ in 0..20 {
        if let Ok(contents) = fs::read_to_string(output_path) {
            output = contents;
            if output == "yes\n" {
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(output, "yes\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_named_coproc_executes_while_body() {
    let output_path = "target/rubash-coproc-while-body-output.txt";
    let status_path = "target/rubash-coproc-while-body-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "coproc MYC while true; do echo loop > {output_path}; break; done; echo pid:${{MYC_PID:+set}} > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert!(coproc.body.as_ref().unwrap()[0].loop_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");

    let mut output = String::new();
    for _ in 0..20 {
        if let Ok(contents) = fs::read_to_string(output_path) {
            output = contents;
            if output == "loop\n" {
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(output, "loop\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_named_coproc_executes_until_body() {
    let output_path = "target/rubash-coproc-until-body-output.txt";
    let status_path = "target/rubash-coproc-until-body-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "coproc MYC until false; do echo loop > {output_path}; break; done; echo pid:${{MYC_PID:+set}} > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let coproc = ast.commands[0].coproc_command.as_ref().unwrap();
    assert!(
        coproc.body.as_ref().unwrap()[0]
            .loop_command
            .as_ref()
            .unwrap()
            .until
    );
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");

    let mut output = String::new();
    for _ in 0..20 {
        if let Ok(contents) = fs::read_to_string(output_path) {
            output = contents;
            if output == "loop\n" {
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(output, "loop\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_named_coproc_executes_time_prefixed_brace_body() {
    let output_path = "target/rubash-coproc-time-brace-output.txt";
    let status_path = "target/rubash-coproc-time-brace-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "coproc MYC time {{ echo timed > {output_path}; }}; echo pid:${{MYC_PID:+set}} > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");

    let mut output = String::new();
    for _ in 0..20 {
        if let Ok(contents) = fs::read_to_string(output_path) {
            output = contents;
            if output == "timed\n" {
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(output, "timed\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_named_coproc_subshell_keeps_case_pattern_parentheses() {
    let output_path = "target/rubash-coproc-subshell-case-output.txt";
    let status_path = "target/rubash-coproc-subshell-case-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "coproc MYC ( case beta in alpha) printf alpha > {output_path} ;; beta) printf beta > {output_path} ;; esac ); echo pid:${{MYC_PID:+set}} > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");

    let mut output = String::new();
    for _ in 0..20 {
        if let Ok(contents) = fs::read_to_string(output_path) {
            output = contents;
            if output == "beta" {
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(output, "beta");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_named_coproc_executes_arithmetic_body_without_stderr() {
    let status_path = "target/rubash-coproc-arithmetic-status.txt";
    let error_path = "target/rubash-coproc-arithmetic-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input =
        format!("coproc MYC (( 1 )) 2> {error_path}; echo pid:${{MYC_PID:+set}} > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");

    let mut error = String::new();
    for _ in 0..20 {
        error = fs::read_to_string(error_path).unwrap_or_default();
        if error.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(error, "");
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_named_coproc_redirects_stderr_to_file() {
    let status_path = "target/rubash-coproc-stderr-redirect-status.txt";
    let error_path = "target/rubash-coproc-stderr-redirect-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "coproc MYC {{ printf coproc-error >&2; }} 2> {error_path}; echo pid:${{MYC_PID:+set}} > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");

    let mut error = String::new();
    for _ in 0..20 {
        error = fs::read_to_string(error_path).unwrap_or_default();
        if error == "coproc-error" {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(error, "coproc-error");
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_named_coproc_redirects_stdout_to_file() {
    let status_path = "target/rubash-coproc-stdout-redirect-status.txt";
    let output_path = "target/rubash-coproc-stdout-redirect-output.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(output_path);
    let input = format!(
        "coproc MYC {{ printf coproc-output; }} > {output_path}; echo pid:${{MYC_PID:+set}} > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "pid:set\n");

    let mut output = String::new();
    for _ in 0..20 {
        output = fs::read_to_string(output_path).unwrap_or_default();
        if output == "coproc-output" {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(output, "coproc-output");
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(output_path);
}
