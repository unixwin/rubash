use super::super::*;
use std::fs;

#[test]
fn test_read_array_n_zero_assigns_empty_array() {
    let output_path = "target/rubash-read-array-n-zero-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "read -a values -n 0 <<< abc; printf '%s:%s' \"${{#values[@]}}\" \"$?\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0:0");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_n_rejects_negative_count() {
    let output_path = "target/rubash-read-n-negative-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -n -1 value <<< abc; echo $? > {output_path}");
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
fn test_read_compact_n_rejects_non_numeric_count() {
    let output_path = "target/rubash-read-n-invalid-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -nabc value <<< abc; echo $? > {output_path}");
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
fn test_read_invalid_counts_redirect_stderr() {
    let output_path = "target/rubash-read-invalid-count-status.txt";
    let error_path = "target/rubash-read-invalid-count-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "read -n -1 value 2> {error_path}; echo negative:$? > {output_path}; \
         read -Nabc value 2>> {error_path}; echo compact:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "negative:1\ncompact:1\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("read: -1: invalid number"));
    assert!(error.contains("read: abc: invalid number"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_read_invalid_options_fail_with_status_two() {
    let output_path = "target/rubash-read-invalid-option-status.txt";
    let error_path = "target/rubash-read-invalid-option-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "read -Z value <<< abc 2> {error_path}; echo simple:$? > {output_path}; \
         read -rsZ value <<< abc 2>> {error_path}; echo compact:$? >> {output_path}; \
         read -E value <<< abc 2>> {error_path}; echo uppercase:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "simple:2\ncompact:2\nuppercase:2\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert_eq!(error.matches("read: -Z: invalid option").count(), 2);
    assert!(error.contains("read: -E: invalid option"));
    assert_eq!(error.matches("read: usage:").count(), 3);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_read_double_dash_allows_name_arguments() {
    let output_path = "target/rubash-read-double-dash-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "read -- value extra <<< 'abc def'; printf '%s:%s:%s' \"$?\" \"$value\" \"$extra\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0:abc:def");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_invalid_scalar_name_reports_identifier_error() {
    let output_path = "target/rubash-read-invalid-scalar-status.txt";
    let error_path = "target/rubash-read-invalid-scalar-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "read ok 1bad later <<< 'a b c' 2> {error_path}; \
         printf '%s:%s:%s' \"$?\" \"$ok\" \"$later\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1:a:");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("read: `1bad': not a valid identifier"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_read_invalid_array_name_falls_back_to_scalar_names() {
    let output_path = "target/rubash-read-invalid-array-status.txt";
    let error_path = "target/rubash-read-invalid-array-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "read -a 1bad ok <<< 'a b' 2> {error_path}; \
         printf '%s:%s' \"$?\" \"$ok\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1:a");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("read: `1bad': not a valid identifier"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_read_a_missing_argument_reports_usage() {
    let output_path = "target/rubash-read-a-missing-status.txt";
    let error_path = "target/rubash-read-a-missing-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("read -a 2> {error_path} <<< 'a b'; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("read: -a: option requires an argument"));
    assert!(error.contains("read: usage:"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_read_combined_rn_reads_raw_limited_characters() {
    let output_path = "target/rubash-read-rn-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -rn3 value <<< 'a\\bcdef'; printf '<%s>' \"$value\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<a\\b>");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_upper_n_reads_through_newline() {
    let output_path = "target/rubash-read-upper-n-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -N 4 value <<< $'ab\\ncd'; printf '<%s>' \"$value\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<ab\nc>");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_r_upper_n_reads_raw_through_newline() {
    let output_path = "target/rubash-read-r-upper-n-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -rN4 value <<< $'ab\\ncd'; printf '<%s>' \"$value\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<ab\nc>");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_upper_n_ignores_delimiter() {
    let output_path = "target/rubash-read-upper-n-delim-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("read -d / -N 5 value <<< abc/def; printf '<%s>' \"$value\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<abc/d>");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_rd_uses_delimiter_and_raw_input() {
    let output_path = "target/rubash-read-rd-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -rd / value <<< 'a\\ b/c'; printf '<%s>' \"$value\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<a\\ b>");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_rd_compact_uses_delimiter_and_raw_input() {
    let output_path = "target/rubash-read-rd-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -rd/ value <<< 'a\\ b/c'; printf '<%s>' \"$value\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<a\\ b>");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_a_splits_here_string_into_array() {
    let output_path = "target/rubash-read-a-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "IFS=, read -a arr <<< 'alpha,beta,gamma'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "3 alpha beta gamma\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_compact_a_uses_attached_array_name() {
    let output_path = "target/rubash-read-a-attached-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("read -aarr <<< 'alpha beta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 alpha beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_a_processes_backslash_escaped_whitespace() {
    let output_path = "target/rubash-read-a-backslash-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -a arr <<< 'a\\ b c'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "3 a b c\n");
    let _ = fs::remove_file(output_path);
}
