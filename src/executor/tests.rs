mod unit_tests {
    use crate::executor::{Executor, SubstitutionQuoteContext};
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
    fn export_assignment_arg_preserves_quoted_spaces() {
        let tokens = tokenize(r#"export PATH="$PATH;C:\Program Files\Tool""#);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("PATH", r"C:\Base");

        executor.execute_ast(&ast).expect("export PATH assignment");

        assert_eq!(
            executor.get_env("PATH"),
            Some(r"C:\Base;C:\Program Files\Tool")
        );
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
    fn mutable_word_typed_backtick_keeps_substitution_status() {
        let mut executor = Executor::new();
        let expanded = executor
            .expand_word_mut_typed_with_context("`printf ok`", SubstitutionQuoteContext::Unquoted)
            .expect("typed backtick word");
        assert_eq!(expanded.status, Some(0));
        assert_eq!(expanded.fragments.len(), 1);
        assert_eq!(expanded.materialize_lossy_at_boundary(), "ok");
    }

    #[test]
    fn command_substitution_lexes_comments_at_word_boundaries() {
        let executor = Executor::new();

        assert_eq!(
            executor.expand_word("$(echo Ok1 #comment is ignored)"),
            "Ok1"
        );
        assert_eq!(
            executor.expand_word("`echo Ok2 #comment is ignored`"),
            "Ok2"
        );
    }

    #[test]
    fn function_pipeline_command_substitution_pipes_stage_output() {
        // Issue #70: the function-call fast path used to run f with
        // `| while ...` as literal positional params instead of piping f's
        // output into the next pipeline stage.
        let tokens = tokenize(
            "f() { echo ARGS=[$@]; }; x=$(f a b | while read -r line; do echo \"GOT:$line\"; done); echo \"x=$x\"",
        );
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.stdout_capture = Some(Vec::new());
        executor
            .execute_ast(&ast)
            .expect("function pipeline comsub");
        let captured = executor.stdout_capture.take().unwrap_or_default();
        let text = String::from_utf8_lossy(&captured);
        assert!(text.contains("x=GOT:ARGS=[a b]"), "got: {text}");
    }

    #[test]
    fn function_pipeline_command_substitution_keeps_plain_call_shortcut() {
        // The single-simple-command shortcut must survive the #70 guard.
        let mut executor = Executor::new();
        let tokens = tokenize("f() { echo ARGS=[$@]; }; x=$(f a b); echo \"x=$x\"");
        let ast = parse(&tokens);
        executor.stdout_capture = Some(Vec::new());
        executor.execute_ast(&ast).expect("plain function comsub");
        let captured = executor.stdout_capture.take().unwrap_or_default();
        let text = String::from_utf8_lossy(&captured);
        assert!(text.contains("x=ARGS=[a b]"), "got: {text}");
    }

    #[test]
    fn heredoc_body_keeps_backslash_before_double_quote() {
        // heredoc.tests: an unquoted heredoc body expands with Q_HERE_DOCUMENT
        // whose escape set is CBSHDOC (subst.c:11628; syntax.h
        // slashify_in_here_document = backslash, backtick, dollar). A double
        // quote is not special in a heredoc body, so a backslash before one is
        // literal data, while double-backslash and backslash-dollar collapse.
        let mut executor = Executor::new();
        let expanded = executor.expand_heredoc_body_mut("echo \\\"\nnext\\\\\nlast\\$v\n");
        assert_eq!(expanded, "echo \\\"\nnext\\\nlast$v\n");
    }

    #[test]
    fn alias_value_heredoc_body_is_not_executed_as_commands() {
        // heredoc10.sub case 1: the alias value itself contains the complete
        // here-document. The deferred-heredoc reparse origin used to drop the
        // body (lexer/mod.rs AliasReplacementDeferredHeredoc) and execute the
        // body lines as commands (exit 127).
        let tokens = tokenize("shopt -s expand_aliases\nalias h='cat <<E\nhello\nworld\nE'\nh\n");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.stdout_capture = Some(Vec::new());
        executor.execute_ast(&ast).expect("alias value heredoc");
        let captured = executor.stdout_capture.take().unwrap_or_default();
        let text = String::from_utf8_lossy(&captured);
        assert_eq!(text, "hello\nworld\n");
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn alias_invocation_heredoc_body_comes_from_following_commands() {
        // heredoc10.sub case 3: the alias value opens the heredoc and the
        // body follows in the outer input.
        let tokens = tokenize("shopt -s expand_aliases\nalias h='cat <<E'\nh\nbody1\nbody2\nE\n");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.stdout_capture = Some(Vec::new());
        executor.execute_ast(&ast).expect("alias heredoc body");
        let captured = executor.stdout_capture.take().unwrap_or_default();
        let text = String::from_utf8_lossy(&captured);
        assert_eq!(text, "body1\nbody2\n");
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn command_substitution_operator_words_are_detected() {
        use crate::executor::command_subst_helpers::command_substitution_words_have_operators;

        let words = |source: &str| crate::executor::split_shell_words(source);
        assert!(command_substitution_words_have_operators(&words(
            "f a b | wc -l"
        )));
        assert!(command_substitution_words_have_operators(&words(
            "f a b 2>/dev/null"
        )));
        assert!(command_substitution_words_have_operators(&words(
            "gitC | sed -e s/a/b/"
        )));
        assert!(!command_substitution_words_have_operators(&words("f a b")));
    }

    #[test]
    fn quoted_command_substitution_preserves_internal_newlines() {
        let tokens = tokenize("echo \"$(printf 'echo foo\\necho bar\\n')\"");
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let expanded = executor
            .expand_command_words(&ast.commands[0])
            .expect("expand command words");

        assert_eq!(
            expanded.words,
            vec!["echo".to_string(), "echo foo\necho bar".to_string()]
        );
    }

    #[test]
    fn unquoted_command_substitution_still_splits_internal_newlines() {
        let tokens = tokenize("echo $(printf 'echo foo\\necho bar\\n')");
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let expanded = executor
            .expand_command_words(&ast.commands[0])
            .expect("expand command words");

        assert_eq!(
            expanded.words,
            vec![
                "echo".to_string(),
                "echo".to_string(),
                "foo".to_string(),
                "echo".to_string(),
                "bar".to_string()
            ]
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

    #[test]
    fn prompt_expansion_runs_starship_ps0_arithmetic_assignment() {
        let mut executor = Executor::new();
        let rendered = executor
            .expand_prompt_string_mut("${STARSHIP_START_TIME:$((STARSHIP_START_TIME=12345,0)):0}");

        assert_eq!(rendered, "");
        assert_eq!(executor.get_env("STARSHIP_START_TIME"), Some("12345"));
    }

    #[test]
    fn prompt_expansion_decodes_raw_escape_markers() {
        let executor = Executor::new();
        let marker = crate::executor::substitution_metadata::encode_raw_byte_marker(0x1b);

        assert_eq!(
            executor.expand_prompt_string(&format!("left{marker}[31mright")),
            "left\x1b[31mright"
        );
    }

    #[test]
    fn shell_identity_is_used_for_default_dollar_zero() {
        let mut executor = Executor::new();
        executor.set_env("__RUBASH_SHELL_NAME", "niu");

        assert_eq!(executor.expand_word("$0"), "niu");
    }

    #[cfg(windows)]
    #[test]
    fn sudo_uses_host_elevation_handler() {
        use crate::executor::{ElevationOutput, SudoMode};
        use std::cell::RefCell;
        use std::rc::Rc;

        let tokens = tokenize("sudo -E --new-window cmd /C echo hi");
        let ast = parse(&tokens);
        let captured = Rc::new(RefCell::new(None));
        let captured_for_handler = Rc::clone(&captured);

        let mut executor = Executor::new();
        executor.export_env("SUDO_TEST_MARKER", "present");
        executor.set_elevation_handler(move |request| {
            *captured_for_handler.borrow_mut() = Some(request.clone());
            Ok(ElevationOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
                status: 7,
            })
        });

        assert!(executor.execute_ast(&ast).is_ok());
        assert_eq!(executor.last_exit_code(), 7);

        let request = captured
            .borrow()
            .clone()
            .expect("sudo should call elevation handler");
        assert_eq!(
            request.command,
            vec![
                "cmd".to_string(),
                "/C".to_string(),
                "echo".to_string(),
                "hi".to_string()
            ]
        );
        assert!(
            request.resolved_program.as_ref().is_some_and(|path| path
                .file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("cmd"))),
            "sudo should resolve target command in the unelevated environment"
        );
        assert_eq!(request.mode, SudoMode::NewWindow);
        assert!(request.preserve_environment);
        assert_eq!(
            request
                .environment
                .get("SUDO_TEST_MARKER")
                .map(String::as_str),
            Some("present")
        );
    }
}
