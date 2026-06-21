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
