use super::super::*;
use std::fs;

#[test]
fn test_dirstack_expands_as_dynamic_array() {
    let output_path = "target/rubash-dirstack-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "PWD=/tmp/rubash-dirstack; \
         printf '%s:%s:%s\\n' \"${{!DIRSTACK[@]}}\" \"${{#DIRSTACK[@]}}\" \"${{DIRSTACK[0]}}\" > {output_path}; \
         DIRSTACK[0]=/tmp/rubash-dirstack-updated; \
         printf '%s:%s:%s\\n' \"${{!DIRSTACK[@]}}\" \"${{#DIRSTACK[@]}}\" \"${{DIRSTACK[0]}}\" >> {output_path}; \
         declare -p DIRSTACK >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0:1:/tmp/rubash-dirstack\n0:1:/tmp/rubash-dirstack-updated\ndeclare -a DIRSTACK=([0]=\"/tmp/rubash-dirstack-updated\")\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_dirs_redirects_stderr() {
    let error_path = "target/rubash-dirs-stderr-output.txt";
    let status_path = "target/rubash-dirs-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("dirs bad 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("dirs: bad: invalid option"));
    assert!(error.contains("dirs: usage:"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_dirs_appends_stderr() {
    let error_path = "target/rubash-dirs-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("dirs bad 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("dirs: bad: invalid option"));
    assert!(error.contains("dirs: usage:"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_pushd_redirects_stderr() {
    let error_path = "target/rubash-pushd-stderr-output.txt";
    let status_path = "target/rubash-pushd-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("pushd +9 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("pushd: +9: directory stack index out of range"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_pushd_n_accepts_double_dash_before_directory() {
    let output_path = "target/rubash-pushd-n-double-dash-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("PWD=/; pushd -n -- /tmp > {output_path}; dirs -p >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "/ /tmp\n/\n/tmp\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_popd_appends_stderr() {
    let error_path = "target/rubash-popd-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("popd +9 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("popd: +9: directory stack index out of range"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_popd_double_dash_ignores_following_stack_index() {
    let output_path = "target/rubash-popd-double-dash-index-output.txt";
    let scratch_path = "target/rubash-popd-double-dash-index-scratch.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(scratch_path);
    let input = format!(
        "PWD=/; pushd /tmp > {scratch_path}; pushd /bin >> {scratch_path}; \
         popd -- +8 > {output_path}; popd -- -8 >> {output_path}; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "/tmp /\n/\n0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(scratch_path);
}

#[test]
fn test_popd_n_removes_next_directory_without_cd() {
    let output_path = "target/rubash-popd-n-output.txt";
    let scratch_path = "target/rubash-popd-n-scratch.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(scratch_path);
    let input = format!(
        "PWD=/; pushd -n /tmp > {scratch_path}; popd -n -- > {output_path}; dirs -p >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "/\n/\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(scratch_path);
}

#[test]
fn test_popd_n_empty_stack_returns_failure() {
    let error_path = "target/rubash-popd-n-empty-error.txt";
    let status_path = "target/rubash-popd-n-empty-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("PWD=/; popd -n 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("popd: directory stack empty"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_kill_redirects_output() {
    let output_path = "target/rubash-kill-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("kill -l HUP > {output_path}");
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
fn test_kill_appends_output() {
    let output_path = "target/rubash-kill-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("kill -l HUP >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "before\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_kill_translates_exit_signal() {
    let output_path = "target/rubash-kill-exit-signal-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("kill -l 0 > {output_path}; kill -l EXIT >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "EXIT\n0\n");
    let _ = fs::remove_file(output_path);
}
