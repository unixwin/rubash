//! Executor Tests - TDD for Bash Command Executor
//!
//! Run with: cargo test --test executor_tests

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

mod simple_execution {
    use super::*;

    #[test]
    fn test_echo_command() {
        let input = "echo hello";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_echo_multiple_args() {
        let input = "echo hello world";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_exit_command() {
        let input = "exit 0";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_err());
    }

    #[test]
    fn test_pwd_command() {
        let input = "pwd";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_true_command() {
        let tokens = tokenize("true");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).ok();
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn test_false_command() {
        let tokens = tokenize("false");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).ok();
        assert_eq!(executor.last_exit_code(), 1);
    }
}

mod exit_codes {
    use super::*;

    #[test]
    fn test_exit_with_code() {
        let input = "exit 42";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).unwrap_err();
        assert_eq!(executor.last_exit_code(), 42);
    }

    #[test]
    fn test_exit_without_code() {
        let input = "exit";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).unwrap_err();
        assert_eq!(executor.last_exit_code(), 0);
    }
}

mod environment_tests {
    use super::*;

    #[test]
    fn test_export_command() {
        let input = "export TEST_VAR=hello";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_env_var_storage() {
        let mut executor = Executor::new();
        executor.set_env("MY_VAR", "hello");
        assert_eq!(executor.get_env("MY_VAR"), Some("hello"));
    }

    #[test]
    fn test_unset_command() {
        let input = "unset HOME";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }
}

mod command_chaining {
    use super::*;
    use std::fs;

    #[test]
    fn test_semicolon_separation() {
        let input = "echo a; echo b";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
        assert_eq!(ast.commands.len(), 2);
    }

    #[test]
    fn test_and_operator() {
        let input = "true && echo success";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_or_operator() {
        let input = "false || echo fallback";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_pipeline_redirects_filtered_output() {
        let output_path = "target/rubash-pipeline-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("echo hello | grep hello > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        assert!(ast.commands[0].pipe.is_some());
        assert_eq!(ast.commands[1].words, ["grep", "hello"]);
        assert!(ast.commands[1].redirect_out.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "hello\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_pipeline_counts_bytes_with_wc() {
        let output_path = "target/rubash-pipeline-wc-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("echo hello | wc -c > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        assert!(ast.commands[0].pipe.is_some());
        assert_eq!(ast.commands[1].words, ["wc", "-c"]);
        assert!(ast.commands[1].redirect_out.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap().trim(), "6");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_pipeline_filters_printf_output() {
        let output_path = "target/rubash-pipeline-printf-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("printf 'a\\nb\\n' | grep b > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        assert!(ast.commands[0].pipe.is_some());
        assert_eq!(ast.commands[0].words, ["printf", "a\\nb\\n"]);
        assert_eq!(ast.commands[1].words, ["grep", "b"]);
        assert!(ast.commands[1].redirect_out.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "b\n");
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
    fn test_pipeline_feeds_external_stage_stdin() {
        let output_path = "target/rubash-pipeline-external-output.txt";
        #[cfg(windows)]
        let script_path = "target/rubash-pipeline-filter.cmd";
        #[cfg(not(windows))]
        let script_path = "target/rubash-pipeline-filter.sh";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(script_path);
        #[cfg(windows)]
        fs::write(
            script_path,
            "@echo off\r\nfor /f \"delims=\" %%L in ('findstr /r \".*\"') do if \"%%L\"==\"b\" echo external:%%L\r\n",
        )
        .unwrap();
        #[cfg(not(windows))]
        fs::write(
            script_path,
            "#!/bin/sh\nwhile IFS= read -r line; do\n  if [ \"$line\" = b ]; then\n    printf 'external:%s\\n' \"$line\"\n  fi\ndone\n",
        )
        .unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(script_path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(script_path, permissions).unwrap();
        }
        let input = format!("printf 'a\\nb\\n' | {script_path} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        assert!(ast.commands[0].pipe.is_some());
        assert_eq!(ast.commands[1].words, [script_path]);
        assert!(ast.commands[1].redirect_out.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path)
                .unwrap()
                .replace("\r\n", "\n"),
            "external:b\n"
        );
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(script_path);
    }

    #[test]
    fn test_mapfile_t_reads_here_string_into_array() {
        let output_path = "target/rubash-mapfile-t-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "mapfile -t arr <<< $'alpha\\nbeta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
        );
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
    fn test_readarray_t_reads_here_string_into_array() {
        let output_path = "target/rubash-readarray-t-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "readarray -t arr <<< $'alpha\\nbeta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
        );
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
    fn test_mapfile_n_limits_read_lines() {
        let output_path = "target/rubash-mapfile-n-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "mapfile -n 2 -t arr <<< $'alpha\\nbeta\\ngamma'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
        );
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
    fn test_readarray_compact_n_limits_read_lines() {
        let output_path = "target/rubash-readarray-compact-n-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "readarray -n1 -t arr <<< $'alpha\\nbeta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1 alpha\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_mapfile_s_skips_initial_lines() {
        let output_path = "target/rubash-mapfile-s-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "mapfile -s 1 -t arr <<< $'alpha\\nbeta\\ngamma'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "2 beta gamma\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_readarray_compact_s_combines_with_n() {
        let output_path = "target/rubash-readarray-compact-s-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "readarray -s1 -n1 -t arr <<< $'alpha\\nbeta\\ngamma'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1 beta\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_read_r_reads_here_string_without_backslash_escape() {
        let output_path = "target/rubash-read-r-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("read -r line <<< 'alpha\\beta'; echo $line > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\\beta\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_read_multiple_names_assigns_remainder_to_last() {
        let output_path = "target/rubash-read-multiple-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("read first rest <<< 'alpha beta gamma'; echo $first:$rest > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "alpha:beta gamma\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_read_multiple_names_uses_custom_ifs() {
        let output_path = "target/rubash-read-ifs-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "IFS=, read first rest <<< 'alpha,beta,gamma'; echo $first:$rest > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "alpha:beta,gamma\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_read_empty_ifs_does_not_split() {
        let output_path = "target/rubash-read-empty-ifs-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("IFS= read first rest <<< 'alpha beta'; echo $first:$rest > {output_path}");
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
    fn test_read_upper_n_reads_through_newline() {
        let output_path = "target/rubash-read-upper-n-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("read -N 4 value <<< $'ab\\ncd'; printf '<%s>' \"$value\" > {output_path}");
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
        assert_eq!(ast.commands[0].words, ["while", "false"]);
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
        assert_eq!(ast.commands[0].words, ["while", "true"]);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "loop\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_until_true_skips_body() {
        let output_path = "target/rubash-until-true-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("until true; do echo bad > {output_path}; done");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands[0].words, ["until", "true"]);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert!(!std::path::Path::new(output_path).exists());
    }

    #[test]
    fn test_case_question_mark_pattern_matches() {
        let output_path = "target/rubash-case-question-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "case hello in h?llo) echo yes > {output_path} ;; *) echo no > {output_path} ;; esac"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert!(ast.commands[0].case_command.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_case_bracket_range_pattern_matches() {
        let output_path = "target/rubash-case-bracket-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "case beta in [a-c]eta) echo yes > {output_path} ;; *) echo no > {output_path} ;; esac"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert!(ast.commands[0].case_command.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_case_fallthrough_executes_next_clause_body() {
        let output_path = "target/rubash-case-fallthrough-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "case beta in alpha) echo alpha > {output_path} ;; beta) echo beta > {output_path} ;& gamma) echo gamma >> {output_path} ;; *) echo star > {output_path} ;; esac"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert!(ast.commands[0].case_command.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "beta\ngamma\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_case_test_next_terminator_matches_later_clause() {
        let output_path = "target/rubash-case-test-next-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "case beta in alpha) echo alpha > {output_path} ;; beta) echo beta > {output_path} ;;& b*) echo bstar >> {output_path} ;; *) echo star > {output_path} ;; esac"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert!(ast.commands[0].case_command.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "beta\nbstar\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_function_keyword_definition_executes_body() {
        let output_path = "target/rubash-function-keyword-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("function greet {{ echo hi > {output_path}; }}; greet");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert!(ast.commands[0].function_command.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "hi\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_for_without_in_iterates_positional_params() {
        let output_path = "target/rubash-for-default-positional-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("set -- alpha beta; for item; do echo $item >> {output_path}; done");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert!(ast.commands[1].for_command.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_for_explicit_empty_in_does_not_iterate() {
        let output_path = "target/rubash-for-empty-in-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("set -- alpha beta; for item in; do echo $item > {output_path}; done");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert!(ast.commands[1].for_command.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert!(!std::path::Path::new(output_path).exists());
    }

    #[test]
    fn test_function_positional_count_expands() {
        let output_path = "target/rubash-function-count-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("function argc {{ echo $# > {output_path}; }}; argc one two three");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "3\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_function_positional_star_expands() {
        let output_path = "target/rubash-function-star-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("function argv {{ echo $* > {output_path}; }}; argv one two three");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "one two three\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_shift_too_many_fails_without_changing_positional_params() {
        let output_path = "target/rubash-shift-too-many-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("function s {{ shift 3; echo $? $# $1 > {output_path}; }}; s one two");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1 2 one\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_shift_non_numeric_fails_without_changing_positional_params() {
        let output_path = "target/rubash-shift-nonnumeric-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("function s {{ shift x; echo $? $# $1 > {output_path}; }}; s one two");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1 2 one\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_shift_negative_fails_without_changing_positional_params() {
        let output_path = "target/rubash-shift-negative-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("function s {{ shift -1; echo $? $# $1 > {output_path}; }}; s one two");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1 2 one\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_function_return_sets_status_and_skips_rest() {
        let output_path = "target/rubash-function-return-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "function r {{ return 7; echo bad > {output_path}; }}; r; echo $? > {output_path}"
        );
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
    fn test_braced_positional_parameters_expand() {
        let output_path = "target/rubash-braced-positional-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("function p {{ echo ${{1}} ${{2}} ${{#}} > {output_path}; }}; p alpha beta");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha beta 2\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_positional_parameter_lengths_expand() {
        let output_path = "target/rubash-positional-length-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "function p {{ echo ${{#1}} ${{#2}} ${{#@}} ${{#*}} > {output_path}; }}; p alpha beta"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "5 4 2 2\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_special_parameter_length_expands() {
        let output_path = "target/rubash-special-length-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("false; echo ${{#?}} > {output_path}");
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
    fn test_parameter_colon_plus_requires_non_empty_value() {
        let output_path = "target/rubash-param-colon-plus-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset v; echo unset:${{v:+alt}} > {output_path}; v=; echo empty:${{v:+alt}} >> {output_path}; v=x; echo set:${{v:+alt}} >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "unset:\nempty:\nset:alt\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_plus_expands_for_empty_set_value() {
        let output_path = "target/rubash-param-plus-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset v; echo unset:${{v+alt}} > {output_path}; v=; echo empty:${{v+alt}} >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "unset:\nempty:alt\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_dash_uses_default_only_when_unset() {
        let output_path = "target/rubash-param-dash-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset v; echo unset:${{v-default}} > {output_path}; v=; echo empty:${{v-default}} >> {output_path}; v=x; echo set:${{v-default}} >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "unset:default\nempty:\nset:x\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_colon_dash_uses_default_for_empty_value() {
        let output_path = "target/rubash-param-colon-dash-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=; echo empty:${{v:-default}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "empty:default\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_equals_assigns_default_only_when_unset() {
        let output_path = "target/rubash-param-equals-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset v; : ${{v=default}}; echo unset:$v > {output_path}; v=; : ${{v=default}}; echo empty:$v >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "unset:default\nempty:\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_equals_assigns_before_regular_command() {
        let output_path = "target/rubash-param-equals-command-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("unset v; echo ${{v=default}} $v > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "default default\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_colon_equals_assigns_empty_value() {
        let output_path = "target/rubash-param-colon-equals-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=; : ${{v:=default}}; echo $v > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "default\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_pattern_removes_prefixes_and_suffixes() {
        let output_path = "target/rubash-param-pattern-remove-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "v=abc_def_ghi; echo ${{v#*_}} ${{v##*_}} ${{v%_*}} ${{v%%_*}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "def_ghi ghi abc_def abc\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_substring_uses_offset_and_length() {
        let output_path = "target/rubash-param-substring-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=abcdef; echo ${{v:2:3}} ${{v:3}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "cde def\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_substring_slices_characters() {
        let output_path = "target/rubash-param-substring-utf8-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=aßcd; echo ${{v:1:2}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "ßc\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_substring_supports_negative_offset() {
        let output_path = "target/rubash-param-substring-negative-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=abcdef; echo ${{v: -2}} ${{v: -4:2}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "ef cd\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_substring_does_not_shadow_colon_dash_default() {
        let output_path = "target/rubash-param-substring-default-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("unset v; echo ${{v:-fallback}} > {output_path}");
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
    fn test_positional_parameter_substring_uses_offset_and_length() {
        let output_path = "target/rubash-positional-substring-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "function p {{ echo ${{@:2:2}} / ${{*:3}} > {output_path}; }}; p alpha beta gamma delta"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "beta gamma / gamma delta\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_positional_parameter_substring_supports_negative_offset() {
        let output_path = "target/rubash-positional-substring-negative-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("function p {{ echo ${{@: -2:1}} > {output_path}; }}; p alpha beta gamma");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "beta\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_replacement_replaces_first_and_all_matches() {
        let output_path = "target/rubash-param-replace-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=banana; echo ${{v/a/o}} ${{v//a/o}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "bonana bonono\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_replacement_deletes_matches() {
        let output_path = "target/rubash-param-replace-delete-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=banana; echo ${{v/a}} ${{v//a}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "bnana bnn\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_replacement_uses_shell_patterns() {
        let output_path = "target/rubash-param-replace-pattern-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=abcd; echo ${{v/?b/X}} ${{v//?/x}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "Xcd xxxx\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_replacement_supports_prefix_and_suffix_anchors() {
        let output_path = "target/rubash-param-replace-anchor-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=abcabc; echo ${{v/#abc/X}} ${{v/%abc/X}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "Xabc abcX\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_replacement_keeps_value_when_anchor_does_not_match() {
        let output_path = "target/rubash-param-replace-anchor-miss-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=abcabc; echo ${{v/#bc/X}} ${{v/%ab/X}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "abcabc abcabc\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_replacement_anchor_uses_shell_patterns() {
        let output_path = "target/rubash-param-replace-anchor-pattern-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=abcd; echo ${{v/#a?/X}} ${{v/%?d/X}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "Xcd abX\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_positional_parameter_replacement_expands_numeric_parameter() {
        let output_path = "target/rubash-positional-replace-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "function p {{ echo ${{1/a/X}} ${{3//m/M}} > {output_path}; }}; p alpha beta gamma"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "Xlpha gaMMa\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_positional_parameter_replacement_expands_all_parameters() {
        let output_path = "target/rubash-positional-replace-all-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("function p {{ echo ${{@/%a/Z}} > {output_path}; }}; p alpha beta gamma");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "alphZ betZ gammZ\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_case_mod_uppercases_first_and_all_chars() {
        let output_path = "target/rubash-param-case-upper-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=hello; echo ${{v^}} ${{v^^}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "Hello HELLO\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_case_mod_lowercases_first_and_all_chars() {
        let output_path = "target/rubash-param-case-lower-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=HELLO; echo ${{v,}} ${{v,,}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "hELLO hello\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_parameter_case_mod_uses_pattern() {
        let output_path = "target/rubash-param-case-pattern-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=abcde; echo ${{v^^[bd]}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "aBcDe\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_indirect_parameter_expands_named_variable() {
        let output_path = "target/rubash-param-indirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("target=value; ref=target; echo ${{!ref}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "value\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_indirect_parameter_uses_positional_parameter_name() {
        let output_path = "target/rubash-param-indirect-positional-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("function p {{ target=value; echo ${{!1}} > {output_path}; }}; p target");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "value\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_indirect_prefix_expands_matching_variable_names() {
        let output_path = "target/rubash-param-indirect-prefix-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "RUBASH_INDIRECT_A=1; RUBASH_INDIRECT_B=2; echo ${{!RUBASH_INDIRECT_*}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "RUBASH_INDIRECT_A RUBASH_INDIRECT_B\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_if_true_executes_then_body() {
        let output_path = "target/rubash-if-true-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("if true; then echo yes > {output_path}; fi");
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
    fn test_elif_true_executes_after_false_if() {
        let output_path = "target/rubash-elif-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("if false; then echo no > {output_path}; elif true; then echo yes > {output_path}; else echo bad > {output_path}; fi");
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
    fn test_if_condition_command_runs_before_then() {
        let condition_path = "target/rubash-if-condition-side-effect.txt";
        let output_path = "target/rubash-if-command-output.txt";
        let _ = fs::remove_file(condition_path);
        let _ = fs::remove_file(output_path);
        let input = format!(
            "if printf cond > {condition_path}; then echo yes > {output_path}; else echo no > {output_path}; fi"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(condition_path).unwrap(), "cond");
        assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\n");
        let _ = fs::remove_file(condition_path);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_if_condition_command_status_selects_else() {
        let output_path = "target/rubash-if-command-false-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("if false; then echo yes > {output_path}; else echo no > {output_path}; fi");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "no\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_if_flattened_arithmetic_comparison_selects_else() {
        let output_path = "target/rubash-if-arith-false-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("if 0 == 1; then echo yes > {output_path}; else echo no > {output_path}; fi");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "no\n");
        let _ = fs::remove_file(output_path);
    }
}

mod builtin_commands {
    use super::*;

    #[test]
    fn test_env_builtin() {
        let input = "env";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_set_builtin() {
        let input = "set";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_test_builtin() {
        let input = "test 1 -eq 1";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_bracket_builtin() {
        let input = "[ 1 -eq 1 ]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }
}
