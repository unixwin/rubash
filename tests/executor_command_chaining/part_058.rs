use super::super::*;
use std::fs;

#[test]
fn test_shift_invalid_counts_redirect_stderr() {
    let error_path = "target/rubash-shift-invalid-stderr.txt";
    let output_path = "target/rubash-shift-invalid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(output_path);
    let input = format!(
        "function s {{ shift x 2> {error_path}; echo nonnumeric:$?:$#:$1 > {output_path}; \
         shift -1 2>> {error_path}; echo negative:$?:$#:$1 >> {output_path}; }}; s one two"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "nonnumeric:1:2:one\nnegative:1:2:one\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("shift: x: numeric argument required"));
    assert!(error.contains("shift: -1: shift count out of range"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shift_too_many_arguments_fails_without_changing_positional_params() {
    let error_path = "target/rubash-shift-too-many-args-stderr.txt";
    let output_path = "target/rubash-shift-too-many-args-output.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(output_path);
    let input = format!(
        "function s {{ shift 1 2 2> {error_path}; echo $? $# $1 > {output_path}; }}; s one two"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1 2 one\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("shift: too many arguments"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shift_verbose_reports_count_out_of_range() {
    let quiet_error_path = "target/rubash-shift-verbose-quiet-error.txt";
    let verbose_error_path = "target/rubash-shift-verbose-error.txt";
    let output_path = "target/rubash-shift-verbose-output.txt";
    for path in [quiet_error_path, verbose_error_path, output_path] {
        let _ = fs::remove_file(path);
    }
    let input = format!(
        "function s {{ shift 3 2> {quiet_error_path}; echo quiet:$?:$#:$1 > {output_path}; \
         shopt -s shift_verbose; shift 3 2> {verbose_error_path}; \
         echo verbose:$?:$#:$1 >> {output_path}; }}; s one two"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "quiet:1:2:one\nverbose:1:2:one\n"
    );
    assert_eq!(fs::read_to_string(quiet_error_path).unwrap(), "");
    assert!(fs::read_to_string(verbose_error_path)
        .unwrap()
        .contains("shift: 3: shift count out of range"));
    for path in [quiet_error_path, verbose_error_path, output_path] {
        let _ = fs::remove_file(path);
    }
}

#[test]
fn test_function_return_sets_status_and_skips_rest() {
    let output_path = "target/rubash-function-return-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("function r {{ return 7; echo bad > {output_path}; }}; r; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "7\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_return_normalizes_status() {
    let output_path = "target/rubash-function-return-normalize-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function r {{ return 258; }}; r; echo $? > {output_path}");
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
fn test_function_return_non_numeric_status_is_usage_error() {
    let output_path = "target/rubash-function-return-bad-status-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "function r {{ return abc; echo bad > {output_path}; }}; r; echo $? > {output_path}"
    );
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
fn test_shell_flags_expand_into_dollar_dash() {
    let output_path = "target/rubash-shell-flags-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -e -x; echo $- > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let flags = fs::read_to_string(output_path).unwrap();
    assert!(flags.contains('e'));
    assert!(flags.contains('x'));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shell_flags_expand_inside_words() {
    let output_path = "target/rubash-shell-flags-embedded-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -u; printf '%s\\n' \"flags:$-\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("flags:"));
    assert!(output.trim_end().contains('u'));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_set_operands_replace_positional_params_after_expansion() {
    let output_path = "target/rubash-set-operands-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("name=beta; set -e alpha $name; echo $# $1 $2 $- > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("2 alpha beta "));
    assert!(output.trim_end().ends_with('e'));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_set_nounset_updates_shell_flags_and_option_tests() {
    let output_path = "target/rubash-set-nounset-flags-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -u; echo $- > {output_path}; [[ -o nounset ]]; echo $? >> {output_path}; \
         set +u; echo $- >> {output_path}; [[ -o nounset ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let lines: Vec<String> = fs::read_to_string(output_path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert!(lines[0].contains('u'));
    assert_eq!(lines[1], "0");
    assert!(!lines[2].contains('u'));
    assert_eq!(lines[3], "1");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_set_nounset_with_positional_operands() {
    let output_path = "target/rubash-set-nounset-operands-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("name=beta; set -u alpha $name; echo $# $1 $2 $- > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.starts_with("2 alpha beta "));
    assert!(output.trim_end().ends_with('u'));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_set_o_nounset_updates_shell_flags() {
    let output_path = "target/rubash-set-o-nounset-flags-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -o nounset; echo $- > {output_path}; test -o nounset");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains('u'));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shellopts_reflects_set_options() {
    let output_path = "target/rubash-shellopts-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "echo $SHELLOPTS > {output_path}; shopt -so physical; echo $SHELLOPTS >> {output_path}; readonly -p >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("braceexpand:hashall:interactive-comments\n"));
    assert!(output.contains("braceexpand:hashall:interactive-comments:physical\n"));
    assert!(output
        .contains("declare -r SHELLOPTS=\"braceexpand:hashall:interactive-comments:physical\""));
    let _ = fs::remove_file(output_path);
}
