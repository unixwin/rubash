use super::super::*;
use std::fs;

#[test]
fn test_read_a_r_treats_backslash_as_literal() {
    let output_path = "target/rubash-read-a-r-backslash-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("read -r -a arr <<< 'a\\ b c'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "3 a\\ b c\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_ra_treats_backslash_as_literal_array() {
    let output_path = "target/rubash-read-ra-backslash-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("read -ra arr <<< 'a\\ b c'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "3 a\\ b c\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_ra_compact_array_name_reads_raw_array() {
    let output_path = "target/rubash-read-ra-compact-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -raarr <<< 'a\\ b c'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "3 a\\ b c\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_sa_reads_array() {
    let output_path = "target/rubash-read-sa-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("read -sa arr <<< 'a\\ b c'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "3 a b c\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_sa_compact_array_name() {
    let output_path = "target/rubash-read-sa-compact-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -saarr <<< 'a\\ b c'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "3 a b c\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_rsa_reads_raw_array() {
    let output_path = "target/rubash-read-rsa-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("read -rsa arr <<< 'a\\ b c'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "3 a\\ b c\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_sra_compact_array_name_reads_raw_array() {
    let output_path = "target/rubash-read-sra-compact-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("read -sraarr <<< 'a\\ b c'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "3 a\\ b c\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_a_processes_backslash_escaped_custom_ifs() {
    let output_path = "target/rubash-read-a-escaped-custom-ifs-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("IFS=, read -a arr <<< 'a\\,b,c'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 a,b c\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_d_limits_array_input_before_delimiter() {
    let output_path = "target/rubash-read-d-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "read -d / -a arr <<< 'one two/three four'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 one two\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_while_false_skips_body() {
    let output_path = "target/rubash-while-false-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("while false; do echo bad > {output_path}; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let loop_command = ast.commands[0].loop_command.as_ref().unwrap();
    assert!(!loop_command.until);
    assert_eq!(loop_command.condition[0].words, ["false"]);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_while_true_runs_until_break() {
    let output_path = "target/rubash-while-break-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("while true; do echo loop > {output_path}; break; done");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let loop_command = ast.commands[0].loop_command.as_ref().unwrap();
    assert!(!loop_command.until);
    assert_eq!(loop_command.condition[0].words, ["true"]);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "loop\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_while_read_consumes_done_redirect_input() {
    let input_path = "target/rubash-while-read-input.txt";
    let output_path = "target/rubash-while-read-redirect-output.txt";
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
    fs::write(input_path, "alpha\nbeta\n").unwrap();
    let input = format!(
        "while read -r; do printf '<%s>\\n' \"$REPLY\" >> {output_path}; done < {input_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<alpha>\n<beta>\n"
    );
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_while_read_or_nonempty_line_condition_list() {
    let output_path = "target/rubash-while-read-or-line-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "count=0; \
         while IFS= read -r line || [ -n \"$line\" ]; do\n\
           case \"$line\" in\n\
             ''|'#'*) continue ;;\n\
             *) count=$((count + 1)); last=$line ;;\n\
           esac\n\
         done <<'EOF'\n# comment\nalpha\nbeta\nEOF\n\
         printf 'count:%s last:%s\\n' \"$count\" \"$last\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "count:2 last:beta\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_return_outside_function_sets_failure_status() {
    let output_path = "target/rubash-return-outside-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("return; echo $? > {output_path}");
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
fn test_return_outside_function_redirects_stderr() {
    let output_path = "target/rubash-return-outside-redirect-output.txt";
    let error_path = "target/rubash-return-outside-redirect-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!("return 3 2> {error_path}; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("return: can only `return' from a function or sourced script"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_return_invalid_number_in_function_returns_two() {
    let output_path = "target/rubash-return-invalid-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("f() {{ return nope; echo bad > {output_path}; }}; f; echo $? > {output_path}");
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
fn test_return_invalid_number_in_function_redirects_stderr() {
    let output_path = "target/rubash-return-invalid-redirect-output.txt";
    let error_path = "target/rubash-return-invalid-redirect-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "f() {{ return nope 2> {error_path}; echo bad > {output_path}; }}; \
         f; echo $? > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("return: nope: numeric argument required"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}
