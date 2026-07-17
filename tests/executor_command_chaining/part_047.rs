use super::super::*;
use std::fs;

#[test]
fn test_read_empty_ifs_does_not_split() {
    let output_path = "target/rubash-read-empty-ifs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("IFS= read first rest <<< 'alpha beta'; echo $first:$rest > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha beta:\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_process_substitution_from_function_output() {
    let output_path = "target/rubash-read-process-subst-function-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "producer() {{ local RUBASH_PS_VALUE=inner; echo a/b/c/d; }}; \
         IFS=/ read first second third rest < <(producer); \
         printf '%s:%s:%s:%s:%s\\n' \"$first\" \"$second\" \"$third\" \"$rest\" \"${{RUBASH_PS_VALUE-unset}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a:b:c:d:unset\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_cat_reads_process_substitution_argument() {
    let output_path = "target/rubash-cat-process-subst-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat <(echo process substitution) > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "process substitution\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_process_substitution_preserves_quoted_printf_escapes() {
    let output_path = "target/rubash-process-subst-printf-quoted-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat <(printf \"x\\n\") > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "x\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_cat_reads_process_substitution_stdin_redirect() {
    let output_path = "target/rubash-cat-process-subst-stdin-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat < <(echo redirected stdin) > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "redirected stdin\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_output_process_substitution_redirect_feeds_command_stdin() {
    let output_path = "target/rubash-output-process-subst-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("echo hi > >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(
        ast.commands[0].redirect_out.as_ref().unwrap().target,
        ">(cat > target/rubash-output-process-subst-redirect-output.txt)"
    );
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hi\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_stderr_process_substitution_redirect_feeds_command_stdin() {
    let output_path = "target/rubash-stderr-process-subst-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -u x value 2> >(cat > {output_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(
        ast.commands[0].redirect_err.as_ref().unwrap().target,
        ">(cat > target/rubash-stderr-process-subst-redirect-output.txt)"
    );
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("read: x: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_output_process_substitution_word_feeds_command_stdin() {
    let input_path = "target/rubash-output-process-subst-word-input.txt";
    let mirror_path = "target/rubash-output-process-subst-word-mirror.txt";
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(mirror_path);
    fs::write(input_path, "data\n").unwrap();
    let input = format!("cp {input_path} >(cat > {mirror_path})");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands[0].words[2], format!(">(cat > {mirror_path})"));
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(mirror_path).unwrap(), "data\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(mirror_path);
}

#[test]
fn test_process_substitution_does_not_see_temporary_assignment() {
    let output_path = "target/rubash-process-subst-temp-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset RUBASH_PS_TEMP; RUBASH_PS_TEMP=inner cat < <(echo $RUBASH_PS_TEMP:1) > {output_path}; \
         echo after:${{RUBASH_PS_TEMP-unset}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        ":1\nafter:unset\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_cat_reads_shell_style_file_path() {
    let output_path = "target/rubash-cat-shell-path-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "file=${{TMPDIR}}/rubash-cat-shell-path-$$; echo shell-path > $file; \
         cat $file > {output_path}; rm -f $file"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "shell-path\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_process_substitution_from_default_parameter_word() {
    let output_path = "target/rubash-default-process-subst-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("value=; cat ${{value:-<(echo fallback)}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "fallback\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_for_loop_body_reads_process_substitution_redirect() {
    let output_path = "target/rubash-for-process-subst-read-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "producer() {{ echo a:b:c:d; }}; \
         for item in once; do IFS=: read first second third rest; done < <(producer); \
         printf '%s-%s-%s-%s\\n' \"$first\" \"$second\" \"$third\" \"$rest\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a-b-c-d\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_process_substitution_keeps_case_pattern_parentheses() {
    let output_path = "target/rubash-process-subst-case-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "cat <(case beta in alpha) printf alpha ;; beta) printf beta ;; esac) > {output_path}"
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
fn test_process_substitution_defers_source_parameter_expansion() {
    let output_path = "target/rubash-process-subst-deferred-expansion-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("cat <(value=inner; echo $value) > {output_path}");
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
fn test_read_without_name_assigns_reply() {
    let output_path = "target/rubash-read-reply-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("unset REPLY; read <<< hello; echo $REPLY > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hello\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_n_reads_limited_characters() {
    let output_path = "target/rubash-read-n-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -n 3 value <<< abcdef; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "abc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_n_compact_option_reads_limited_characters() {
    let output_path = "target/rubash-read-n-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -n3 value <<< abcdef; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "abc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_sn_consumes_limit() {
    let output_path = "target/rubash-read-sn-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -sn 3 value <<< abcdef; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "abc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_en_consumes_limit() {
    let output_path = "target/rubash-read-en-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -en 3 value <<< abcdef; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "abc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_en_compact_limit() {
    let output_path = "target/rubash-read-en-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -en3 value <<< abcdef; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "abc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_ren_consumes_limit_and_reads_raw() {
    let output_path = "target/rubash-read-ren-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -ren 4 value <<< 'a\\bcd'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\\bc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_ern_compact_limit_reads_raw() {
    let output_path = "target/rubash-read-ern-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -ern4 value <<< 'a\\bcd'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\\bc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_s_upper_n_compact_limits_characters() {
    let output_path = "target/rubash-read-s-upper-n-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -sN4 value <<< abcdef; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "abcd\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_e_upper_n_consumes_limit() {
    let output_path = "target/rubash-read-e-upper-n-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -eN 4 value <<< abcdef; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "abcd\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_e_upper_n_compact_limit() {
    let output_path = "target/rubash-read-e-upper-n-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -eN4 value <<< abcdef; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "abcd\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_re_upper_n_consumes_limit_and_reads_raw() {
    let output_path = "target/rubash-read-re-upper-n-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -reN 4 value <<< 'a\\bcd'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\\bc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_combined_er_upper_n_compact_limit_reads_raw() {
    let output_path = "target/rubash-read-er-upper-n-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -erN4 value <<< 'a\\bcd'; echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\\bc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_n_zero_succeeds_with_empty_value() {
    let output_path = "target/rubash-read-n-zero-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -n 0 value <<< abc; printf '<%s>:%s' \"$value\" $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<>:0");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_read_upper_n_zero_succeeds_with_empty_value() {
    let output_path = "target/rubash-read-upper-n-zero-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("read -N 0 value <<< abc; printf '<%s>:%s' \"$value\" $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<>:0");
    let _ = fs::remove_file(output_path);
}
