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
    fn test_pipeline_feeds_external_stage_stdin() {
        let output_path = "target/rubash-pipeline-external-output.txt";
        let script_path = "target/rubash-pipeline-filter.sh";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(script_path);
        fs::write(
            script_path,
            "while IFS= read -r line; do\n  if [ \"$line\" = b ]; then\n    printf 'external:%s\\n' \"$line\"\n  fi\ndone\n",
        )
        .unwrap();
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
        assert_eq!(fs::read_to_string(output_path).unwrap(), "external:b\n");
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
