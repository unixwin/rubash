use super::super::*;
use std::fs;

#[test]
fn test_quoted_heredoc_keeps_parameters_literal() {
    let output_path = "target/rubash-quoted-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("value=expanded; cat > {output_path} <<'EOF'\n$value\nEOF");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "$value\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_spaced_quoted_heredoc_keeps_parameters_literal() {
    let output_path = "target/rubash-spaced-quoted-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("value=expanded; cat > {output_path} << 'EOF'\n$value\nEOF");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "$value\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_partially_quoted_heredoc_delimiter_keeps_parameters_literal() {
    let output_path = "target/rubash-partial-quoted-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("value=expanded; cat > {output_path} <<E\"OF\"\n$value\nEOF");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "$value\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_multiple_heredocs_use_last_stdin_redirect() {
    let output_path = "target/rubash-multiple-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat > {output_path} <<EOF1 <<EOF2\nfirst\nEOF1\nsecond\nEOF2");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "second\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_dash_heredoc_strips_leading_tabs() {
    let output_path = "target/rubash-dash-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat > {output_path} <<-EOF\n\tone\n\ttwo\n\tEOF");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "one\ntwo\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_dash_quoted_tab_delimiter_strips_leading_tabs() {
    let output_path = "target/rubash-dash-quoted-tab-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("cat > {output_path} <<-'\tEND'\n\thello\n\tEND\necho after >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hello\nafter\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_empty_quoted_heredoc_delimiter_reads_until_eof() {
    let output_path = "target/rubash-empty-quoted-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat > {output_path} <<''\nhi\nthere\n''");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hi\nthere\n''\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unquoted_heredoc_removes_backslash_newline() {
    let output_path = "target/rubash-heredoc-backslash-newline-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat > {output_path} <<EOF\nline 1\\\nline 2\nEOF");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "line 1line 2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_heredoc_delimiter_word_removes_backslash_newline() {
    let output_path = "target/rubash-heredoc-delimiter-continuation-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat > {output_path} <<EO\\\nF\nhi\nEOF");
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
fn test_unquoted_heredoc_matches_delimiter_after_backslash_newline() {
    let output_path = "target/rubash-heredoc-body-delimiter-continuation-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat > {output_path} <<EOF\nhi\nEO\\\nF");
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
fn test_unquoted_heredoc_body_continuation_before_delimiter_check() {
    let output_path = "target/rubash-heredoc-body-continuation-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat > {output_path} <<EOF\nnext\\\nEOF\nEOF");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "nextEOF\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unterminated_subshell_heredoc_does_not_execute_body() {
    let output_path = "target/rubash-unterminated-subshell-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("(cat <<EOF > {output_path}\nstill more text in a subshell\nEOF)");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_err());
    assert_eq!(executor.last_exit_code(), 2);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_for_loop_heredoc_append_expands_loop_variable_target() {
    let dir = target_test_path("rubash-for-heredoc-append");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let shell_dir = shell_test_path(&dir);
    let input =
        format!("cd {shell_dir}; for f in a b c; do cat <<-EOF >> ${{f}}\n\tfile\n\tEOF\ndone");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(dir.join("a")).unwrap(), "file\n");
    assert_eq!(fs::read_to_string(dir.join("b")).unwrap(), "file\n");
    assert_eq!(fs::read_to_string(dir.join("c")).unwrap(), "file\n");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_while_loop_reads_stdin_and_fd_heredocs() {
    let output_path = "target/rubash-while-fd-heredoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "while read line1; do read line2 <&3; echo $line1 - $line2 >> {output_path}; done <<EOF1 3<<EOF2\none\ntwo\nthree\nEOF1\nalpha\nbeta\ngamma\nEOF2"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "one - alpha\ntwo - beta\nthree - gamma\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_lineno_assignment_does_not_override_dynamic_value() {
    let output_path = "target/rubash-lineno-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("LINENO=99; printf '%s\\n' \"$LINENO\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}
