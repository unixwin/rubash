use super::super::*;
use std::fs;

#[test]
fn test_local_compound_array_assignment_preserves_quoted_array_at() {
    let output_path = "target/rubash-local-compound-array-quoted-at-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() {{ foo=(a 'b c' d); local v=(\"${{foo[@]}}\"); printf '<%s>\\n' \"${{v[@]}}\" > {output_path}; }}; f"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<a>\n<b c>\n<d>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_pipeline_feeds_external_stage_stdin() {
    let output_path = "target/rubash-function-pipeline-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("emit() {{ printf '%s\\n' a 'b c'; }}; emit | tr '\\n' ',' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a,b c,");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_definition_pipeline_stage_is_isolated() {
    let output_path = "target/rubash-function-definition-pipeline-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "emit() {{ echo body; }} | cat > {output_path}; \
         emit >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let pipeline = ast.commands[0].pipeline_command.as_ref().unwrap();
    assert!(pipeline.stages[0].function_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 127);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_decimal_regex_does_not_match_negative_integer() {
    let output_path = "target/rubash-decimal-regex-negative-integer-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=-9; if [[ \"$n\" =~ ^[-]?([0-9]*)\\.([0-9]+)$ ]]; then echo decimal > {output_path}; elif [[ \"$n\" =~ ^[-]?[0-9]+$ ]]; then echo integer:${{BASH_REMATCH[0]}} > {output_path}; fi"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "integer:-9\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_decimal_split_regex_keeps_literal_dot() {
    let output_path = "target/rubash-decimal-split-regex-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=1.3; [[ \"$value\" =~ (.*)\\.(.*) ]]; echo \"${{BASH_REMATCH[1]}}/${{BASH_REMATCH[2]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1/3\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_regex_no_match_clears_bash_rematch() {
    let output_path = "target/rubash-regex-no-match-rematch-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "[[ abc =~ (a)(b)(c) ]]; [[ xx =~ z+ ]]; echo \"${{BASH_REMATCH[0]:-empty}}/${{BASH_REMATCH[1]:-empty}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "empty/empty\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_positional_slice_expands_to_multiple_arguments() {
    let output_path = "target/rubash-quoted-positional-slice-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() {{ printf '<%s>\\n' \"${{@:1:2}}\" > {output_path}; printf 'tail:<%s>\\n' \"${{@:3}}\" >> {output_path}; }}; f a b c d"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<a>\n<b>\ntail:<c>\ntail:<d>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_array_at_slice_expands_to_multiple_arguments() {
    let output_path = "target/rubash-quoted-array-at-slice-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one 'two words' three); \
         printf 'at<%s>\\n' \"${{arr[@]:1:2}}\" > {output_path}; \
         printf 'star<%s>\\n' \"${{arr[*]:1:2}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "at<one>\nat<two words>\nstar<one two words>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_array_star_slice_uses_ifs_first_char() {
    let output_path = "target/rubash-quoted-array-star-slice-ifs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "IFS=,; arr=(zero one 'two words' three); \
         printf 'star<%s>\\n' \"${{arr[*]:1:2}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "star<one,two words>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unquoted_array_expansion_field_splits_values() {
    let output_path = "target/rubash-unquoted-array-field-split-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one 'two words' three); \
         printf 'all<%s>\\n' ${{arr[@]}} > {output_path}; \
         printf 'slice<%s>\\n' ${{arr[@]:1:2}} >> {output_path}; \
         printf 'star<%s>\\n' ${{arr[*]:1:2}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "all<zero>\nall<one>\nall<two>\nall<words>\nall<three>\nslice<one>\nslice<two>\nslice<words>\nstar<one>\nstar<two>\nstar<words>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_slice_negative_length_reports_error() {
    let output_path = "target/rubash-array-negative-length-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "s=abcd; arr=(zero one two three); \
         printf 'scalar<%s>\\n' \"${{s:1:-1}}\" > {output_path}; \
         printf 'array<%s>\\n' \"${{arr[@]:1:-1}}\" >> {output_path}; \
         echo after >> {output_path}"
    );
    // wt37 (#374): run through the real CLI - the in-process tokenize+execute_ast
    // posture cannot model this construct; the CLI run is byte-identical to WSL GNU
    // Bash 5.3.0 (probe wt37-374 K001, target/issue-suites/results/wt37-374/run/).
    let (_cli_out, cli_err, cli_code) = run_cli_script(&input);
    assert_eq!(cli_code, Some(1), "stderr: {cli_err}");

    assert_eq!(fs::read_to_string(output_path).unwrap(), "scalar<bc>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_positional_at_expands_to_multiple_arguments() {
    let output_path = "target/rubash-quoted-positional-at-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("f() {{ printf '<%s>\\n' \"$@\" > {output_path}; }}; f a 'b c' d");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<a>\n<b c>\n<d>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_printf_percent_n_assigns_output_count() {
    let output_path = "target/rubash-printf-percent-n-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("printf 'abc%n:%s' COUNT done > {output_path}; echo $COUNT >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "abc:done3\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_printf_percent_n_with_v_assignment() {
    let output_path = "target/rubash-printf-percent-n-v-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("printf -v OUT 'ab%ncd' COUNT; echo $OUT:$COUNT > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "abcd:2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_printf_v_assigns_indexed_array_element() {
    let output_path = "target/rubash-printf-v-indexed-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("printf -v arr[1] '%s' value; echo \"${{arr[1]}}:${{#arr[@]}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "value:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_indexed_array_state_is_shared_with_printf() {
    let output_path = "target/rubash-declare-printf-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -a arr=(zero one); printf -v arr[3] '%s' three; \
         printf '%s:%s:%s\\n' \"${{arr[0]}}\" \"${{arr[1]}}\" \"${{arr[3]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "zero:one:three\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_printf_v_indexed_array_target_uses_arithmetic_subscript() {
    let output_path = "target/rubash-printf-v-indexed-array-arith-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one two); i=1; printf -v arr[i+1] '%s' last; \
         printf '%s:%s:%s\\n' \"${{arr[0]}}\" \"${{arr[1]}}\" \"${{arr[2]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "zero:one:last\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_printf_v_indexed_array_target_resolves_negative_subscript() {
    let output_path = "target/rubash-printf-v-indexed-array-negative-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one two); printf -v arr[-1] '%s' last; \
         printf '%s:%s:%s\\n' \"${{arr[0]}}\" \"${{arr[1]}}\" \"${{arr[2]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "zero:one:last\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_printf_v_assigns_associative_array_element() {
    let output_path = "target/rubash-printf-v-assoc-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -A m; printf -v m[key] '%s' value; echo \"${{m[key]}}:${{#m[@]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "value:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_printf_time_format_uses_posix_timezone_rules() {
    let output_path = "target/rubash-printf-time-format-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "TZ=EST5EDT,M3.2.0/2,M11.1.0/2 printf '%()T|%(%e-%b-%Y %T)T|%(%F %r %z %Z)T\\n' 1275250155 1275250155 0 > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "16:09:15|30-May-2010 16:09:15|1969-12-31 07:00:00 PM -0500 EST\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_printf_time_format_honors_width_precision_and_embedded_parens() {
    let output_path = "target/rubash-printf-time-width-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "TZ=EST5EDT,M3.2.0/2,M11.1.0/2 printf '%-40.50(%x (foo) %X)T date-style time\\n' 1275250155 > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "05/30/10 (foo) 16:09:15                  date-style time\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_printf_time_format_without_argument_uses_current_time() {
    let output_path = "target/rubash-printf-time-current-output.txt";
    let status_path = "target/rubash-printf-time-current-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let input = format!("TZ=UTC printf '%(%s)T' > {output_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let rendered: i64 = fs::read_to_string(output_path)
        .unwrap()
        .parse()
        .expect("current epoch seconds");
    assert!((before..=after).contains(&rendered));
    assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}
