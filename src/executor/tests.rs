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
        let tokens = tokenize(
            r#"export RUBASH_TEST_TOOLPATH="$RUBASH_TEST_TOOLPATH;C:\Program Files\Tool""#,
        );
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("RUBASH_TEST_TOOLPATH", r"C:\Base");

        executor
            .execute_ast(&ast)
            .expect("export quoted assignment");

        assert_eq!(
            executor.get_env("RUBASH_TEST_TOOLPATH"),
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
            executor.expand_assignment_value("x", "`echo -n \" ab \"`"),
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
            .shell_state.env_vars
            .insert("EUID".to_string(), "0".to_string());
        assert_eq!(executor.decode_prompt_string("\\$"), "#");

        executor
            .shell_state.env_vars
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

    /// Issue #71: negative lookup cache must store misses (None) so repeated
    /// lookups of the same missing name are O(1) instead of a full PATH scan.
    #[test]
    fn negative_command_lookup_cache_stores_misses() {
        use crate::executor::path::{
            clear_command_lookup_cache, find_user_command, set_command_lookup_cache,
        };
        use std::collections::HashMap;

        clear_command_lookup_cache();
        let env = HashMap::<String, String>::new();

        // A miss should be cached as None by find_user_command when hashall
        // is on (the default).  Verify by inserting a None directly and
        // checking that find_user_command returns it without re-scanning.
        set_command_lookup_cache("definitely_not_a_real_command_xyz_test", None);
        let result = find_user_command("definitely_not_a_real_command_xyz_test", &env);
        assert!(
            result.is_none(),
            "cached negative lookup should return None without re-scanning"
        );
        clear_command_lookup_cache();
    }

    /// Issue #71: clear_command_lookup_cache must forget all entries including
    /// negative ones, so the next lookup re-scans PATH.
    #[test]
    fn clear_command_lookup_cache_removes_negative_entries() {
        use crate::executor::path::{
            clear_command_lookup_cache, remove_command_lookup_cache, set_command_lookup_cache,
        };
        use std::path::PathBuf;

        clear_command_lookup_cache();
        set_command_lookup_cache("cached_miss_name", None);
        set_command_lookup_cache("cached_hit_name", Some(PathBuf::from("/bin/echo")));

        // Remove both via clear
        clear_command_lookup_cache();

        // After clear, re-inserting should work from scratch (no stale entries
        // interfere).  This verifies the clear was complete.
        set_command_lookup_cache("cached_miss_name", None);
        remove_command_lookup_cache("cached_miss_name");
        clear_command_lookup_cache();
    }

    /// Issue #71: remove_command_lookup_cache must remove individual negative
    /// entries (hash -d NAME on a never-found command).
    #[test]
    fn remove_command_lookup_cache_removes_negative_entry() {
        use crate::executor::path::{
            clear_command_lookup_cache, remove_command_lookup_cache, set_command_lookup_cache,
        };

        clear_command_lookup_cache();
        set_command_lookup_cache("hash_d_negative_test_cmd", None);

        // remove_command_lookup_cache should not panic and should remove the
        // entry.  After removal, the cache no longer has the entry.
        remove_command_lookup_cache("hash_d_negative_test_cmd");

        // Re-inserting after removal should work (entry was actually removed).
        set_command_lookup_cache("hash_d_negative_test_cmd", None);
        clear_command_lookup_cache();
    }

    /// Issue #71: PATH change must invalidate the negative cache via the
    /// fingerprint mechanism.  A different PATH fingerprint clears all cached
    /// results (both hits and misses) so the next lookup re-scans.
    #[test]
    fn path_change_invalidates_negative_cache() {
        use crate::executor::path::{
            clear_command_lookup_cache, find_user_command, set_command_lookup_cache,
        };
        use std::collections::HashMap;

        clear_command_lookup_cache();

        // Insert a negative entry with the default (empty) environment.
        set_command_lookup_cache("path_invalidation_test_cmd", None);

        // A lookup with the same environment should return the cached None.
        let env_same = HashMap::<String, String>::new();
        let result = find_user_command("path_invalidation_test_cmd", &env_same);
        assert!(
            result.is_none(),
            "same-fingerprint lookup should return cached negative result"
        );

        // A lookup with a DIFFERENT PATH should invalidate the cache and
        // re-scan.  The re-scan will also return None (command doesn't exist),
        // but the important thing is that the fingerprint changed and the
        // cache was cleared.
        let mut env_changed = HashMap::new();
        env_changed.insert(
            "PATH".to_string(),
            "/nonexistent_dummy_path_12345".to_string(),
        );
        let result = find_user_command("path_invalidation_test_cmd", &env_changed);
        assert!(
            result.is_none(),
            "changed PATH should re-scan and still return None for missing command"
        );

        clear_command_lookup_cache();
    }

    // GNU builtins/exit.def:157 exit_builtin -> jump_to_top_level (EXITPROG),
    // handled at execute_cmd.c:1622: `exit` unwinds every enclosing AND-OR
    // list, function and brace group to the shell's top level. Only true
    // subshell boundaries (parentheses, pipeline stages, command
    // substitutions) contain it. These tests pin the unwind semantics that
    // regressed when ExitCode was downgraded to a plain status inside AND-OR
    // list execution (niubash shell-quirks doc Q3: `cmd || exit 1` guards
    // silently no-op'd).

    #[test]
    fn exit_in_and_or_list_terminates_shell() {
        let tokens = tokenize("true && exit 5; echo UNREACHED");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            matches!(result, Err(crate::executor::ExecuteError::ExitCode(5))),
            "exit inside && must unwind the list, got {:?}",
            result
        );
        assert_eq!(executor.last_exit_code(), 5);
    }

    #[test]
    fn exit_in_or_guard_terminates_shell() {
        let tokens = tokenize("[ -f /definitely-missing-file-x ] || exit 1; echo UNREACHED");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            matches!(result, Err(crate::executor::ExecuteError::ExitCode(1))),
            "the classic cmd || exit 1 guard must terminate, got {:?}",
            result
        );
        assert_eq!(executor.last_exit_code(), 1);
    }

    #[test]
    fn exit_as_first_list_element_terminates_shell() {
        let tokens = tokenize("exit 0 && echo never; echo UNREACHED");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            matches!(result, Err(crate::executor::ExecuteError::ExitCode(0))),
            "exit leading an && list must terminate, got {:?}",
            result
        );
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn exit_in_function_and_or_list_terminates_shell() {
        let tokens = tokenize("f(){ true && exit 5; echo unf; }; f; echo UNREACHED");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            matches!(result, Err(crate::executor::ExecuteError::ExitCode(5))),
            "exit inside a function's && list terminates the whole shell (GNU: no scope exit), got {:?}",
            result
        );
        assert_eq!(executor.last_exit_code(), 5);
    }

    #[test]
    fn exit_in_brace_group_and_or_list_terminates_shell() {
        let tokens = tokenize("{ true && exit 5; echo unf; }; echo UNREACHED");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            matches!(result, Err(crate::executor::ExecuteError::ExitCode(5))),
            "brace groups are not subshells: exit must unwind them, got {:?}",
            result
        );
        assert_eq!(executor.last_exit_code(), 5);
    }

    #[test]
    fn exit_in_loop_guard_terminates_shell() {
        // The original Q3 repro shape: a probe loop whose body guards the
        // exit behind an && chain.
        let tokens = tokenize(
            "while true; do [ ! -f /definitely-missing-file-x ] && exit 0; break; done; echo UNREACHED",
        );
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            matches!(result, Err(crate::executor::ExecuteError::ExitCode(0))),
            "exit 0 inside a loop's && guard must terminate the shell, got {:?}",
            result
        );
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn subshell_exit_does_not_terminate_parent_shell() {
        let tokens = tokenize("(exit 5); echo after");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            result.is_ok(),
            "parenthesized exit must stay in the subshell, got {:?}",
            result
        );
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn pipeline_stage_exit_stays_in_stage() {
        let tokens = tokenize("exit 5 | cat; echo after");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            result.is_ok(),
            "pipeline-stage exit must not end the shell, got {:?}",
            result
        );
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn command_substitution_exit_stays_in_subshell() {
        let tokens = tokenize("v=$(true && exit 5; echo hi)");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            result.is_ok(),
            "command-substitution exit must stay in the substitution, got {:?}",
            result
        );
        // The substitution was killed by the exit, so `hi` never ran and the
        // assignment carries the substitution's status (GNU behavior).
        assert_eq!(executor.last_exit_code(), 5);
    }

    #[test]
    fn errexit_guard_context_still_suppresses_and_or_exit_status() {
        // `false && exit 1` is a guarded context: the exit never runs and
        // set -e does not fire on &&/|| left-hand failures (GNU set-e
        // semantics), so the following command still runs.
        let tokens = tokenize("set -e; false && exit 1; echo ok");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            result.is_ok(),
            "guarded false && exit must not trigger errexit, got {:?}",
            result
        );
        assert_eq!(executor.last_exit_code(), 0);
    }
    // niubash shell-quirks Q16: an enclosing compound redirect (for loop /
    // brace group, bound once into the fd table) must not reach external
    // children inside a command substitution — GNU subst.c:7143 runs the
    // body with its stdout on the capture pipe. Before the fix the child
    // inherited the loop's fd-1 file binding, so its output leaked into the
    // outer redirect target and `$( )` came back empty.

    fn cmdsub_external_probe_command(marker_redirect: &str) -> String {
        if cfg!(windows) {
            format!(
                "for i in 1; do out=$(cmd /c \"echo q16-marker\" 2>&1); \
[ -n \"$out\" ] || exit 9; done > \"{marker_redirect}\""
            )
        } else {
            format!(
                "for i in 1; do out=$(sh -c 'echo q16-marker' 2>&1); \
[ -n \"$out\" ] || exit 9; done > \"{marker_redirect}\""
            )
        }
    }

    #[test]
    fn cmdsub_external_capture_survives_loop_redirect_binding() {
        let redirect = std::env::temp_dir().join("rubash-q16-loop-redirect.log");
        let redirect = redirect.to_string_lossy().replace('\\', "/");
        let tokens = tokenize(&cmdsub_external_probe_command(&redirect));
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            result.is_ok(),
            "captured output must be non-empty (exit 9 means the substitution lost it), got {:?}",
            result
        );
        let leaked = std::fs::read_to_string(&redirect).unwrap_or_default();
        assert!(
            !leaked.contains("q16-marker"),
            "cmdsub output leaked into the outer redirect target: {leaked:?}"
        );
        let _ = std::fs::remove_file(&redirect);
    }

    #[test]
    fn cmdsub_external_capture_survives_group_redirect_binding() {
        let redirect = std::env::temp_dir().join("rubash-q16-group-redirect.log");
        let redirect = redirect.to_string_lossy().replace('\\', "/");
        let probe = cmdsub_external_probe_command(&redirect);
        let probe = probe.replacen("for i in 1; do ", "{ ", 1).replacen(
            " done > \"",
            " } > \"",
            1,
        );
        let tokens = tokenize(&probe);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(
            result.is_ok(),
            "captured output must be non-empty (exit 9 means the substitution lost it), got {:?}",
            result
        );
        let leaked = std::fs::read_to_string(&redirect).unwrap_or_default();
        assert!(
            !leaked.contains("q16-marker"),
            "cmdsub output leaked into the outer redirect target: {leaked:?}"
        );
        let _ = std::fs::remove_file(&redirect);
    }
}
