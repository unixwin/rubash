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
