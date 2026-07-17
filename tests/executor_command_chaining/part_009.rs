use super::super::*;
use std::fs;

#[test]
fn test_bash_lineno_and_source_track_function_stack() {
    let output_path = "target/rubash-bash-lineno-source-stack-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "outer() {{ inner; }}; \
         inner() {{ printf '%s|%s|%s|%s|%s\\n' \"${{BASH_LINENO[0]}}\" \"${{BASH_LINENO[1]}}\" \"${{BASH_SOURCE[0]}}\" \"${{BASH_SOURCE[1]}}\" \"${{#BASH_SOURCE[@]}}\" > {output_path}; }}; \
         outer"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("__RUBASH_SCRIPT_NAME", "./stack-source.tests");

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "1|1|./stack-source.tests|./stack-source.tests|3\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_argc_and_argv_track_function_arguments() {
    let output_path = "target/rubash-bash-argc-argv-stack-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "outer() {{ inner c; }}; \
         inner() {{ \
             printf 'argc:%s:%s argv:%s:%s\\n' \"${{BASH_ARGC[0]}}\" \"${{BASH_ARGC[1]}}\" \"${{BASH_ARGV[0]}}\" \"${{BASH_ARGV[1]}}\" > {output_path}; \
             printf '<%s>\\n' \"${{BASH_ARGV[@]}}\" >> {output_path}; \
         }}; \
         outer a b"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "argc:1:2 argv:c:b\n<c>\n<b>\n<a>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_funcname_assignment_does_not_change_stack() {
    let output_path = "target/rubash-funcname-noassign-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "show_name() {{ FUNCNAME[0]=bad; declare FUNCNAME=also_bad; printf '%s:%s:%s\\n' \"$?\" \"$FUNCNAME\" \"${{FUNCNAME[0]}}\" > {output_path}; }}; show_name"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0:show_name:show_name\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_funcname_assignment_does_not_override_dynamic_value() {
    let output_path = "target/rubash-funcname-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("FUNCNAME=42; printf '<%s>\\n' \"$FUNCNAME\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_groups_expands_as_dynamic_array() {
    let output_path = "target/rubash-groups-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "printf '%s:%s:%s\\n' \"$GROUPS\" \"${{GROUPS[0]}}\" \"${{#GROUPS[@]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0:0:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_groups_at_expands_as_dynamic_array_words() {
    let output_path = "target/rubash-groups-at-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '<%s>\\n' \"${{GROUPS[@]}}\" \"${{GROUPS[*]}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<0>\n<0>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_groups_assignment_does_not_override_dynamic_array() {
    let output_path = "target/rubash-groups-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "GROUPS[0]=-1; status=$?; printf '%s:%s\\n' \"$status\" \"${{GROUPS[0]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_lineno_expands_to_current_command_line() {
    let output_path = "target/rubash-lineno-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf '%s:%s\\n' \"$LINENO\" \"$((LINENO))\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_lineno_in_multiline_function_body_uses_body_line() {
    let output_path = "target/rubash-lineno-function-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("show_line() {{\n  printf '%s\\n' \"$LINENO\" > {output_path}\n}}\nshow_line");
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
fn test_unquoted_empty_parameter_removes_word() {
    let output_path = "target/rubash-unquoted-empty-parameter-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("empty=; echo a $empty b > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a b\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_cat_heredoc_redirects_to_output_file() {
    let output_path = "target/rubash-cat-heredoc-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat > {output_path} <<'EOF'\nalpha\nbeta\nEOF");
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
fn test_dev_null_output_redirect_allows_following_commands() {
    let output_path = "target/rubash-dev-null-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("echo hidden > /dev/null; echo visible > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "visible\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_touch_posix_literal_glob_filename_does_not_abort_script() {
    let output_path = "target/rubash-touch-literal-glob-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("touch 'x*x'; echo ok > {output_path}; rm 'x*x'");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unquoted_heredoc_expands_parameters() {
    let output_path = "target/rubash-unquoted-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("value=expanded; cat > {output_path} <<EOF\n$value\nEOF");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "expanded\n");
    let _ = fs::remove_file(output_path);
}
