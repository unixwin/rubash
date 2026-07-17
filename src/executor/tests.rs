mod unit_tests {
    use crate::executor::Executor;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    #[test]
    fn test_execute_echo() {
        let tokens = tokenize("echo hello");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        assert!(executor.execute_ast(&ast).is_ok());
    }

    #[test]
    fn test_exit_code() {
        let tokens = tokenize("exit 5");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_err());
        assert_eq!(executor.last_exit_code(), 5);
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
    fn test_colon_command() {
        let tokens = tokenize(":");
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

    #[test]
    fn test_env_var() {
        let mut executor = Executor::new();
        executor.set_env("TEST_VAR", "hello");
        assert_eq!(executor.get_env("TEST_VAR"), Some("hello"));
    }

    #[test]
    fn backtick_command_substitution_splits_newlines() {
        let executor = Executor::new();

        assert_eq!(executor.expand_word("`echo 'foo\nbar'`"), "foo bar");
    }

    #[test]
    fn assignment_backtick_command_substitution_preserves_spaces() {
        let mut executor = Executor::new();

        assert_eq!(
            executor.expand_assignment_value("`echo -n \" ab \"`"),
            " ab "
        );
    }

    #[test]
    fn prompt_dollar_escape_uses_effective_uid() {
        let mut executor = Executor::new();

        executor
            .env_vars
            .insert("EUID".to_string(), "0".to_string());
        assert_eq!(executor.decode_prompt_string("\\$"), "#");

        executor
            .env_vars
            .insert("EUID".to_string(), "1000".to_string());
        assert_eq!(executor.decode_prompt_string("\\$"), "$");
    }
}
