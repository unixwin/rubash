use super::super::*;
use std::fs;

#[test]
fn test_export_appends_stderr() {
    let error_path = "target/rubash-export-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("export -Z 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("rubash: export: -Z: invalid option"));
    assert!(error.contains("export: usage:"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_readonly_redirects_stderr() {
    let error_path = "target/rubash-readonly-stderr-output.txt";
    let status_path = "target/rubash-readonly-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("readonly -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("readonly: -Z: invalid option"));
    assert!(error.contains("readonly: usage:"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_readonly_appends_stderr() {
    let error_path = "target/rubash-readonly-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("readonly -Z 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("readonly: -Z: invalid option"));
    assert!(error.contains("readonly: usage:"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_export_p_appends_output() {
    let output_path = "target/rubash-export-p-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("export RUBASH_EXPORT_APPEND=value; export -p >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("before\n"));
    assert!(output.contains("declare -x RUBASH_EXPORT_APPEND=\"value\"\n"));
    std::env::remove_var("RUBASH_EXPORT_APPEND");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pwd_redirects_output() {
    let output_path = "target/rubash-pwd-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("PWD=/tmp/rubash-pwd-test pwd > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "/tmp/rubash-pwd-test\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pwd_appends_output() {
    let output_path = "target/rubash-pwd-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("PWD=/tmp/rubash-pwd-test pwd >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\n/tmp/rubash-pwd-test\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pwd_redirects_stderr() {
    let error_path = "target/rubash-pwd-stderr-output.txt";
    let status_path = "target/rubash-pwd-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);

    let input = format!("pwd -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("rubash: pwd: -x: invalid option"));
    assert!(error.contains("pwd: usage: pwd [-LP]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_pwd_appends_stderr() {
    let error_path = "target/rubash-pwd-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();

    let input = format!("pwd -x 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("rubash: pwd: -x: invalid option"));
    assert!(error.contains("pwd: usage: pwd [-LP]"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_pwd_preserves_ordered_output_redirects() {
    let output_path = "target/rubash-pwd-ordered-output.txt";
    let _ = fs::remove_file(output_path);

    let input = format!("PWD=/tmp/rubash-pwd-test pwd >&2 2> {output_path}; test -e {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(fs::metadata(output_path).is_ok());
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_export_preserves_ordered_output_redirects() {
    let output_path = "target/rubash-export-ordered-output.txt";
    let _ = fs::remove_file(output_path);

    let input = format!("export RUBASH_EXPORT_ORDERED=value; export -p >&2 2> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(fs::metadata(output_path).is_ok());
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    std::env::remove_var("RUBASH_EXPORT_ORDERED");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_readonly_preserves_ordered_diagnostic_redirects() {
    let error_path = "target/rubash-readonly-ordered-error.txt";
    let _ = fs::remove_file(error_path);

    let input = format!("readonly -Z >&2 2> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("readonly: -Z: invalid option"));
    assert!(error.contains("readonly: usage:"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_option_builtins_preserve_ordered_output_redirects() {
    for (name, command) in [
        ("shopt", "shopt -p sourcepath"),
        ("umask", "umask"),
        ("enable", "enable -a"),
    ] {
        let output_path = format!("target/rubash-{name}-ordered-output.txt");
        let _ = fs::remove_file(&output_path);
        let input = format!("{command} >&2 2> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok(), "{name} execution failed: {result:?}");
        assert!(fs::metadata(&output_path).is_ok(), "{name} target missing");
        assert_eq!(
            fs::read_to_string(&output_path).unwrap(),
            "",
            "{name} output bypassed the ordered redirect"
        );
        let _ = fs::remove_file(output_path);
    }
}

#[test]
fn test_misc_io_builtins_preserve_ordered_output_redirects() {
    for (name, command) in [
        ("declare", "declare -p PATH"),
        ("help", "help cd"),
        ("kill", "kill -l"),
        ("set", "set -o"),
        ("times", "times"),
        ("ulimit", "ulimit -a"),
    ] {
        let output_path = format!("target/rubash-{name}-ordered-output.txt");
        let _ = fs::remove_file(&output_path);
        let input = format!("{command} >&2 2> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok(), "{name} execution failed: {result:?}");
        assert!(fs::metadata(&output_path).is_ok(), "{name} target missing");
        assert_eq!(
            fs::read_to_string(&output_path).unwrap(),
            "",
            "{name} output bypassed the ordered redirect"
        );
        let _ = fs::remove_file(output_path);
    }
}

#[test]
fn test_alias_preserves_ordered_output_redirects() {
    let output_path = "target/rubash-alias-ordered-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "alias rubash_ordered_alias='echo value'; alias rubash_ordered_alias >&2 2> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok(), "alias execution failed: {result:?}");
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_hash_redirects_output() {
    let output_path = "target/rubash-hash-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("hash -p /tmp/rubash-cat cat; hash -t cat > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "/tmp/rubash-cat\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_hash_appends_output() {
    let output_path = "target/rubash-hash-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("hash -p /tmp/rubash-cat cat; hash -t cat >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\n/tmp/rubash-cat\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_hash_empty_table_reports_success() {
    let error_path = "target/rubash-hash-empty-stderr-output.txt";
    let status_path = "target/rubash-hash-empty-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("hash 2> {error_path}; echo $? > {status_path}");
    // wt37 (#374): run through the real CLI - the in-process tokenize+execute_ast
    // posture cannot model this construct; the CLI run is byte-identical to WSL GNU
    // Bash 5.3.0 (probe wt37-374 K021, target/issue-suites/results/wt37-374/run/).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(0), "stderr: {cli_err}");

    // GNU 5.3.0 `hash` on an empty table prints `hash: hash table empty` to
    // stdout; the `2>` redirection still creates the (empty) error file on
    // both shells (probe wt37-374 K021).
    assert_eq!(_cli_out, "hash: hash table empty\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    assert_eq!(fs::read_to_string(error_path).unwrap(), "");
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_function_and_or_left_command_keeps_heredoc_body() {
    let output_path = "target/rubash-function-and-or-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("foo () {{ cat <<EOF > {output_path} && {{ echo \"$1\"; }}\n$1\nEOF\n}}\nfoo bar");
    let ast = parse(&tokenize(&input));
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(
        result.is_ok(),
        "function heredoc execution failed: {result:?}"
    );
    assert_eq!(fs::read_to_string(output_path).unwrap(), "bar\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_redirect_failure_sets_status_without_stopping_script() {
    let status_path = "target/rubash-redirect-failure-status.txt";
    let _ = fs::remove_file(status_path);
    let missing_parent = "target/rubash-redirect-failure-missing-parent/output.txt";
    let input = format!("a=`` > {missing_parent}; echo $? > {status_path}");
    let ast = parse(&tokenize(&input));
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(
        result.is_ok(),
        "redirect failure aborted script: {result:?}"
    );
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_empty_command_substitution_word_is_removed() {
    let status_path = "target/rubash-empty-command-substitution-status.txt";
    let value_path = "target/rubash-empty-command-substitution-value.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(value_path);
    let input = format!(
        "v=v; v=`exit 2` `false`; echo Two:$? v:\"[$v]\" > {value_path}; echo $? > {status_path}"
    );
    let ast = parse(&tokenize(&input));
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok(), "empty substitution word failed: {result:?}");
    assert_eq!(fs::read_to_string(value_path).unwrap(), "Two:2 v:[]\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(value_path);
}
