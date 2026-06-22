//! Executor Tests - TDD for Bash Command Executor
//!
//! Run with: cargo test --test executor_tests

use rubash::executor::{ExecuteError, Executor};
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
    fn test_mapfile_o_sets_origin_index() {
        let input = "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta'";
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            executor.get_env("arr"),
            Some("\x1d([2]=\"alpha\" [3]=\"beta\")")
        );
    }

    #[test]
    fn test_readarray_compact_o_preserves_existing_elements() {
        let input = "arr=(zero one two); readarray -O2 -n1 -t arr <<< $'new\\nmore'";
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            executor.get_env("arr"),
            Some("\x1d([0]=\"zero\" [1]=\"one\" [2]=\"new\")")
        );
    }

    #[test]
    fn test_mapfile_d_uses_custom_delimiter() {
        let input = "mapfile -d : -t arr <<< 'alpha:beta:gamma'";
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            executor.get_env("arr"),
            Some("\x1d([0]=\"alpha\" [1]=\"beta\" [2]=$'gamma\\n')")
        );
    }

    #[test]
    fn test_readarray_compact_d_keeps_delimiter_without_t() {
        let input = "readarray -d: arr <<< 'alpha:beta'";
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            executor.get_env("arr"),
            Some("\x1d([0]=\"alpha:\" [1]=$'beta\\n')")
        );
    }

    #[test]
    fn test_array_at_indices_expand() {
        let output_path = "target/rubash-array-at-indices-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta'; echo ${{!arr[@]}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "2 3\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_array_star_indices_expand() {
        let output_path = "target/rubash-array-star-indices-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta'; echo ${{!arr[*]}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "2 3\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_array_numeric_subscript_expands_element() {
        let output_path = "target/rubash-array-subscript-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("arr=(zero one two); echo ${{arr[1]}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "one\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_sparse_array_numeric_subscript_expands_element() {
        let output_path = "target/rubash-sparse-array-subscript-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta'; echo ${{arr[2]}} ${{arr[3]}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha beta\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_array_numeric_subscript_length_expands() {
        let output_path = "target/rubash-array-subscript-length-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta'; echo ${{#arr[2]}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "5\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_assoc_array_subscript_expands_element() {
        let output_path = "target/rubash-assoc-subscript-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("declare -A assoc; assoc[one]=alpha; assoc[two]=beta; echo ${{assoc[one]}} ${{assoc[two]}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha beta\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_assoc_array_subscript_length_expands() {
        let output_path = "target/rubash-assoc-subscript-length-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("declare -A assoc; assoc[one]=alpha; echo ${{#assoc[one]}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "5\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_assoc_array_indices_expand_keys() {
        let output_path = "target/rubash-assoc-indices-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("declare -A assoc; assoc[one]=alpha; assoc[two]=beta; echo ${{!assoc[@]}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "one two\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_unset_indexed_array_element() {
        let output_path = "target/rubash-unset-array-element-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "arr=(zero one two); unset 'arr[1]'; echo ${{!arr[@]}} / ${{arr[@]}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0 2 / zero two\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_unset_assoc_array_element() {
        let output_path = "target/rubash-unset-assoc-element-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("declare -A assoc; assoc[one]=alpha; assoc[two]=beta; unset 'assoc[one]'; echo ${{!assoc[@]}} > {output_path}; echo ${{assoc[one]}} >> {output_path}; echo ${{assoc[two]}} >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "two\n\nbeta\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_declare_p_redirects_output() {
        let output_path = "target/rubash-declare-p-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=value; declare -p v > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "declare -- v=\"value\"\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_declare_p_appends_output() {
        let output_path = "target/rubash-declare-p-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("v=value; declare -p v >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "before\ndeclare -- v=\"value\"\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_export_p_redirects_output() {
        let output_path = "target/rubash-export-p-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("export RUBASH_EXPORT_REDIR=value; export -p > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert!(fs::read_to_string(output_path)
            .unwrap()
            .contains("declare -x RUBASH_EXPORT_REDIR=\"value\"\n"));
        std::env::remove_var("RUBASH_EXPORT_REDIR");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_readonly_p_redirects_output() {
        let output_path = "target/rubash-readonly-p-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("readonly RUBASH_READONLY_REDIR=value; readonly -p > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "declare -r RUBASH_READONLY_REDIR=\"value\"\n"
        );
        std::env::remove_var("RUBASH_READONLY_REDIR");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_export_p_appends_output() {
        let output_path = "target/rubash-export-p-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("export RUBASH_EXPORT_APPEND=value; export -p >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("before\n"));
        assert!(output.contains("declare -x RUBASH_EXPORT_APPEND=\"value\"\n"));
        std::env::remove_var("RUBASH_EXPORT_APPEND");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_pwd_redirects_output() {
        let output_path = "target/rubash-pwd-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("PWD=/tmp/rubash-pwd-test pwd > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "/tmp/rubash-pwd-test\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_pwd_appends_output() {
        let output_path = "target/rubash-pwd-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("PWD=/tmp/rubash-pwd-test pwd >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "before\n/tmp/rubash-pwd-test\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_hash_redirects_output() {
        let output_path = "target/rubash-hash-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("hash -p /tmp/rubash-cat cat; hash -t cat > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "/tmp/rubash-cat\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_hash_appends_output() {
        let output_path = "target/rubash-hash-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("hash -p /tmp/rubash-cat cat; hash -t cat >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "before\n/tmp/rubash-cat\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_shopt_redirects_output() {
        let output_path = "target/rubash-shopt-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("shopt -p sourcepath > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "shopt -s sourcepath\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_shopt_appends_output() {
        let output_path = "target/rubash-shopt-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("shopt -p sourcepath >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "before\nshopt -s sourcepath\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_umask_redirects_output() {
        let output_path = "target/rubash-umask-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("umask 077; umask > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0077\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_umask_appends_output() {
        let output_path = "target/rubash-umask-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("umask 077; umask >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "before\n0077\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_times_redirects_output() {
        let output_path = "target/rubash-times-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("times > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_times_appends_output() {
        let output_path = "target/rubash-times-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("times >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "before\n0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_help_redirects_output() {
        let output_path = "target/rubash-help-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("help -s help > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "help: help [-dms] [pattern ...]\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_help_appends_output() {
        let output_path = "target/rubash-help-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("help -s help >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "before\nhelp: help [-dms] [pattern ...]\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_dirs_redirects_output() {
        let output_path = "target/rubash-dirs-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("PWD=/tmp/rubash-dirs dirs > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "/tmp/rubash-dirs\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_dirs_appends_output() {
        let output_path = "target/rubash-dirs-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("PWD=/tmp/rubash-dirs dirs >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "before\n/tmp/rubash-dirs\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_kill_redirects_output() {
        let output_path = "target/rubash-kill-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("kill -l HUP > {output_path}");
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
    fn test_kill_appends_output() {
        let output_path = "target/rubash-kill-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("kill -l HUP >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "before\n1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_ulimit_redirects_output() {
        let output_path = "target/rubash-ulimit-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("ulimit -n > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1024\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_ulimit_appends_output() {
        let output_path = "target/rubash-ulimit-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("ulimit -n >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "before\n1024\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_alias_redirects_output() {
        let output_path = "target/rubash-alias-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("alias ll='ls -l'; alias -p > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "alias ll='ls -l'\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_alias_appends_output() {
        let output_path = "target/rubash-alias-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("alias ll='ls -l'; alias -p >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "before\nalias ll='ls -l'\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_set_redirects_output() {
        let output_path = "target/rubash-set-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("RUBASH_SET_REDIRECT=value set > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("RUBASH_SET_REDIRECT=value\n"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_set_appends_output() {
        let output_path = "target/rubash-set-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("RUBASH_SET_APPEND=value set >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("before\n"));
        assert!(output.contains("RUBASH_SET_APPEND=value\n"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_enable_redirects_output() {
        let output_path = "target/rubash-enable-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("enable -ps > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("enable break\n"));
        assert!(output.contains("enable times\n"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_enable_appends_output() {
        let output_path = "target/rubash-enable-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("enable -ps >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("before\n"));
        assert!(output.contains("enable break\n"));
        assert!(output.contains("enable times\n"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_echo_redirects_output() {
        let output_path = "target/rubash-builtin-echo-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin echo hello > {output_path}");
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
    fn test_builtin_echo_appends_output() {
        let output_path = "target/rubash-builtin-echo-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("builtin echo hello >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "before\nhello\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_command_echo_redirects_output() {
        let output_path = "target/rubash-command-echo-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("command echo hello > {output_path}");
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
    fn test_command_echo_appends_output() {
        let output_path = "target/rubash-command-echo-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("command echo hello >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "before\nhello\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_shift_help_redirects_output_and_returns_usage() {
        let output_path = "target/rubash-shift-help-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("shift --help > {output_path}; echo $? >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("shift: shift [n]\n"));
        assert!(output.contains("Shift positional parameters."));
        assert!(output.ends_with("2\n"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_shift_help_appends_output_and_returns_usage() {
        let output_path = "target/rubash-shift-help-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("shift --help >> {output_path}; echo $? >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("before\nshift: shift [n]\n"));
        assert!(output.contains("Shift positional parameters."));
        assert!(output.ends_with("2\n"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_exit_help_redirects_output_and_exits_usage() {
        let output_path = "target/rubash-exit-help-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("exit --help > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(2))));
        assert_eq!(executor.last_exit_code(), 2);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("exit: exit [n]\n"));
        assert!(output.contains("Exit the shell."));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_exit_help_appends_output_and_exits_usage() {
        let output_path = "target/rubash-exit-help-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("exit --help >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(2))));
        assert_eq!(executor.last_exit_code(), 2);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("before\nexit: exit [n]\n"));
        assert!(output.contains("Exit the shell."));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_eval_redirects_entire_output() {
        let output_path = "target/rubash-eval-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "old\n").unwrap();
        let input = format!("eval 'echo alpha; echo beta' > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\nbeta\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_eval_appends_entire_output() {
        let output_path = "target/rubash-eval-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("eval 'echo alpha; echo beta' >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "before\nalpha\nbeta\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_eval_redirects_loop_body_without_retruncating() {
        let output_path = "target/rubash-eval-loop-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("eval 'for x in a b; do echo $x; done' > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "a\nb\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_type_redirects_output() {
        let output_path = "target/rubash-type-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("type -t echo > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "builtin\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_type_appends_output() {
        let output_path = "target/rubash-type-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("type -t echo >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "before\nbuiltin\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_command_v_redirects_output() {
        let output_path = "target/rubash-command-v-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("command -v echo > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "echo\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_type_a_reports_builtin_and_path_matches() {
        let bin_dir = "target/rubash-type-a-bin";
        let echo_path = format!("{bin_dir}/echo");
        let output_path = "target/rubash-type-a-output.txt";
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&echo_path, "").unwrap();
        let _ = fs::remove_file(output_path);
        let input = format!("type -a echo > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("echo is a shell builtin\n"));
        assert!(output.contains("echo is target/rubash-type-a-bin/echo\n"));
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(echo_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_type_ap_reports_only_path_matches() {
        let bin_dir = "target/rubash-type-ap-bin";
        let echo_path = format!("{bin_dir}/echo");
        let output_path = "target/rubash-type-ap-output.txt";
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&echo_path, "").unwrap();
        let _ = fs::remove_file(output_path);
        let input = format!("type -ap echo > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "target/rubash-type-ap-bin/echo\n"
        );
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(echo_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_type_f_skips_shell_functions() {
        let output_path = "target/rubash-type-f-function-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("function f {{ echo hi; }}; type -f f; echo $? > {output_path}");
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
    fn test_type_f_still_reports_builtins() {
        let output_path = "target/rubash-type-f-builtin-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("type -f echo > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "echo is a shell builtin\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_type_long_type_option_reports_kind() {
        let output_path = "target/rubash-type-long-type-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("type --type echo > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "builtin\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_type_long_all_option_reports_all_matches() {
        let bin_dir = "target/rubash-type-long-all-bin";
        let echo_path = format!("{bin_dir}/echo");
        let output_path = "target/rubash-type-long-all-output.txt";
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&echo_path, "").unwrap();
        let _ = fs::remove_file(output_path);
        let input = format!("type -all echo > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("echo is a shell builtin\n"));
        assert!(output.contains("echo is target/rubash-type-long-all-bin/echo\n"));
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(echo_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_trap_p_redirects_saved_exit_trap() {
        let output_path = "target/rubash-trap-p-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("trap 'echo bye' EXIT; trap -p EXIT > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "trap -- 'echo bye' EXIT\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_trap_reset_removes_saved_trap() {
        let output_path = "target/rubash-trap-reset-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("trap 'echo bye' EXIT; trap - EXIT; trap -p EXIT > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_trap_ignore_appends_saved_signal_trap() {
        let output_path = "target/rubash-trap-ignore-append-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("trap '' INT; trap -p INT >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "before\ntrap -- '' SIGINT\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_exit_runs_exit_trap_and_preserves_status() {
        let output_path = "target/rubash-exit-trap-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("trap 'echo bye > {output_path}' EXIT; exit 7");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(7))));
        assert_eq!(executor.last_exit_code(), 7);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "bye\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_exit_trap_exit_overrides_status() {
        let input = "trap 'exit 3' EXIT; exit 7";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(3))));
        assert_eq!(executor.last_exit_code(), 3);
    }

    #[test]
    fn test_normal_completion_runs_exit_trap() {
        let output_path = "target/rubash-normal-exit-trap-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("trap 'echo done > {output_path}' EXIT; true");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);
        let status = executor.run_exit_trap();

        assert!(result.is_ok());
        assert!(status.is_ok());
        assert_eq!(status.unwrap(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "done\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_exit_trap_sees_last_status_on_normal_completion() {
        let output_path = "target/rubash-normal-exit-trap-status-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("trap 'echo $? > {output_path}' EXIT; false");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);
        let status = executor.run_exit_trap();

        assert!(result.is_ok());
        assert!(status.is_ok());
        assert_eq!(status.unwrap(), 1);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_trap_invalid_signal_returns_failure() {
        let output_path = "target/rubash-trap-invalid-signal-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("trap 'echo bad' NO_SUCH_SIGNAL; echo $? > {output_path}");
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
    fn test_trap_missing_signal_spec_returns_usage() {
        let output_path = "target/rubash-trap-missing-signal-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("trap 512; echo $? > {output_path}");
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
    fn test_trap_l_redirects_signal_list() {
        let output_path = "target/rubash-trap-l-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("trap -l > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with(" 1) SIGHUP"));
        assert!(output.contains("15) SIGTERM"));
        assert!(output.contains("64) SIGRTMAX"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_trap_lp_lists_signals_and_returns_success() {
        let output_path = "target/rubash-trap-lp-status-output.txt";
        let list_path = "target/rubash-trap-lp-list-output.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(list_path);
        let input = format!("trap -lp > {list_path}; echo $? > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(list_path);
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
    fn test_break_outside_loop_returns_success() {
        let output_path = "target/rubash-break-outside-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("break not-a-number; echo $? > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_break_zero_in_loop_returns_failure_without_breaking() {
        let output_path = "target/rubash-break-zero-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("while true; do break 0; echo $? > {output_path}; break; done");
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
    fn test_break_accepts_positive_signed_level() {
        let output_path = "target/rubash-break-plus-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "while true; do break +1; echo bad > {output_path}; done; echo ok > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_continue_zero_in_loop_returns_failure_without_continuing() {
        let output_path = "target/rubash-continue-zero-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("while true; do continue 0; echo $? > {output_path}; break; done");
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
    fn test_break_two_exits_nested_loops() {
        let output_path = "target/rubash-break-two-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "for outer in a b; do for inner in c d; do break 2; echo inner >> {output_path}; done; echo outer >> {output_path}; done; echo ok > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
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
    fn test_parameter_colon_question_errors_for_unset_value() {
        let output_path = "target/rubash-param-colon-question-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("unset v; echo ${{v:?boom}} > {output_path}; echo after > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
        assert_eq!(executor.last_exit_code(), 1);
        assert!(!std::path::Path::new(output_path).exists());
    }

    #[test]
    fn test_parameter_question_allows_empty_set_value() {
        let output_path = "target/rubash-param-question-empty-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("v=; echo ok:${{v?boom}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "ok:\n");
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
    fn test_array_parameter_substring_uses_offset_and_length() {
        let output_path = "target/rubash-array-substring-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("arr=(zero one two three); echo ${{arr[@]:1:2}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "one two\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_array_parameter_substring_supports_negative_offset() {
        let output_path = "target/rubash-array-substring-negative-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("arr=(zero one two three); echo ${{arr[*]: -2}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "two three\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_sparse_array_parameter_substring_slices_values() {
        let output_path = "target/rubash-sparse-array-substring-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta\\ngamma'; echo ${{arr[@]:1:1}} > {output_path}"
        );
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
    fn test_array_parameter_replacement_expands_elements() {
        let output_path = "target/rubash-array-replace-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("arr=(banana gamma); echo ${{arr[@]/a/o}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "bonana gomma\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_array_parameter_replacement_expands_all_matches() {
        let output_path = "target/rubash-array-replace-all-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("arr=(banana gamma); echo ${{arr[*]//a/o}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "bonono gommo\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_array_parameter_replacement_deletes_matches() {
        let output_path = "target/rubash-array-replace-delete-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("arr=(banana gamma); echo ${{arr[@]//a}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "bnn gmm\n");
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
    fn test_array_parameter_case_mod_expands_elements() {
        let output_path = "target/rubash-array-case-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("arr=(alpha beta); echo ${{arr[@]^^}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "ALPHA BETA\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_array_parameter_case_mod_uses_pattern() {
        let output_path = "target/rubash-array-case-pattern-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("arr=(ALPHA BETA); echo ${{arr[*],,[PT]}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "ALpHA BEtA\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_positional_parameter_case_mod_expands_elements() {
        let output_path = "target/rubash-positional-case-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("function p {{ echo ${{@^^}} / ${{1,,}} > {output_path}; }}; p alpha BETA");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "ALPHA BETA / alpha\n"
        );
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
    fn test_indirect_array_parameter_transform_expands_first_value() {
        let output_path = "target/rubash-param-indirect-array-transform-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "arr=(alpha beta); ref=arr; echo ${{!ref[@]@Q}} ${{!ref[*]@U}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha ALPHA\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_indirect_array_pattern_removes_prefixes_and_suffixes() {
        let output_path = "target/rubash-param-indirect-array-pattern-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "arr=(aaa bbb); ref='arr[@]'; echo ${{!ref##aa}} > {output_path}; echo ${{!ref[@]%b}} >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "a bbb\naaa bb\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_test_v_checks_array_subscripts() {
        let output_path = "target/rubash-test-v-array-subscript-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "v=value; arr=(zero one); declare -A assoc; assoc[one]=alpha; test -v 'v[0]'; echo $? > {output_path}; test -v 'v[1]'; echo $? >> {output_path}; test -v 'arr[1]'; echo $? >> {output_path}; test -v 'arr[9]'; echo $? >> {output_path}; test -v 'assoc[one]'; echo $? >> {output_path}; test -v 'assoc[two]'; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0\n1\n0\n1\n0\n1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_conditional_v_checks_array_subscripts() {
        let output_path = "target/rubash-conditional-v-array-subscript-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "v=value; arr=(zero one); declare -A assoc; assoc[one]=alpha; [[ -v v[0] ]]; echo $? > {output_path}; [[ -v v[1] ]]; echo $? >> {output_path}; [[ -v arr[1] ]]; echo $? >> {output_path}; [[ -v arr[9] ]]; echo $? >> {output_path}; [[ -v assoc[one] ]]; echo $? >> {output_path}; [[ -v assoc[two] ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0\n1\n0\n1\n0\n1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_conditional_string_unary_checks_expanded_value() {
        let output_path = "target/rubash-conditional-string-unary-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "value=abc; empty=; [[ -n abc ]]; echo $? > {output_path}; [[ -n $empty ]]; echo $? >> {output_path}; [[ -z abc ]]; echo $? >> {output_path}; [[ -z $empty ]]; echo $? >> {output_path}; [[ -n $value ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n1\n0\n0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_conditional_binary_checks_expand_values() {
        let output_path = "target/rubash-conditional-binary-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "left=abc; right=def; n=3; [[ $left = abc ]]; echo $? > {output_path}; [[ $left != $right ]]; echo $? >> {output_path}; [[ $n -ne 4 ]]; echo $? >> {output_path}; [[ $n -lt 4 ]]; echo $? >> {output_path}; [[ $n -le 3 ]]; echo $? >> {output_path}; [[ $n -ge 3 ]]; echo $? >> {output_path}; [[ $n -gt 4 ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0\n0\n0\n0\n0\n0\n1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_conditional_numeric_checks_evaluate_arithmetic_expressions() {
        let output_path = "target/rubash-conditional-arithmetic-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=3; [[ n+2*4 -eq 11 ]]; echo $? > {output_path}; [[ $n*2 -ge 6 ]]; echo $? >> {output_path}; [[ -5+2 -lt 0 ]]; echo $? >> {output_path}; [[ n/0 -eq 0 ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n0\n0\n1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_status_uses_expression_value() {
        let output_path = "target/rubash-arithmetic-command-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=3; (( 0 )); echo $? > {output_path}; (( n + 1 )); echo $? >> {output_path}; (( n - 3 )); echo $? >> {output_path}; (( n * 2 )); echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n1\n0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_updates_variables() {
        let output_path = "target/rubash-arithmetic-command-updates-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=0; (( n++ )); echo $? $n > {output_path}; (( ++n )); echo $? $n >> {output_path}; (( n += 3 )); echo $? $n >> {output_path}; (( n = 0 )); echo $? $n >> {output_path}; (( n /= 0 )); echo $? $n >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "1 1\n0 2\n0 5\n1 0\n1 0\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_let_builtin_evaluates_arithmetic_expressions() {
        let output_path = "target/rubash-let-arithmetic-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "let n=1 n+=2 n; echo $? $n > {output_path}; let n=0; echo $? $n >> {output_path}; let n=2 n**=3 n-8; echo $? $n >> {output_path}; let n/=0; echo $? $n >> {output_path}; let; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0 3\n1 0\n1 8\n1 8\n1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_comma_sequences_evaluate_in_order() {
        let output_path = "target/rubash-arithmetic-command-comma-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=0; (( n = 1, n += 2, n )); echo $? $n > {output_path}; (( n++, n++, n - 5 )); echo $? $n >> {output_path}; (( n = 0, n )); echo $? $n >> {output_path}; (( (1, 2) )); echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0 3\n1 5\n1 0\n0\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_comparison_operators() {
        let output_path = "target/rubash-arithmetic-command-comparison-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=3; (( n > 2 )); echo $? > {output_path}; (( n < 2 )); echo $? >> {output_path}; (( n >= 3 )); echo $? >> {output_path}; (( n <= 2 )); echo $? >> {output_path}; (( n == 3 )); echo $? >> {output_path}; (( n != 3 )); echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0\n1\n0\n1\n0\n1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_logical_operators() {
        let output_path = "target/rubash-arithmetic-command-logical-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=3; (( n > 2 && n < 4 )); echo $? > {output_path}; (( n > 2 && n < 3 )); echo $? >> {output_path}; (( n > 5 || n == 3 )); echo $? >> {output_path}; (( n > 5 || n < 0 )); echo $? >> {output_path}; (( !0 )); echo $? >> {output_path}; (( !n )); echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0\n1\n0\n1\n0\n1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_logical_operators_short_circuit() {
        let output_path = "target/rubash-arithmetic-command-short-circuit-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "(( 1 || 1 / 0 )); echo $? > {output_path}; (( 0 && 1 / 0 )); echo $? >> {output_path}; (( 0 && 1 / 0 || 4 )); echo $? >> {output_path}; (( 1 || (1 / 0), 0 )); echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n0\n1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_bitwise_and_shift_operators() {
        let output_path = "target/rubash-arithmetic-command-bitwise-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=6; (( n & 2 )); echo $? > {output_path}; (( n & 1 )); echo $? >> {output_path}; (( 1 << 3 | 2 )); echo $? >> {output_path}; (( 14 >> 2 )); echo $? >> {output_path}; [[ 5^3 -eq 6 ]]; echo $? >> {output_path}; (( ~0 + 1 )); echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0\n1\n0\n0\n0\n1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_exponentiation_operators() {
        let output_path = "target/rubash-arithmetic-command-exponent-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=2; (( n ** 3 )); echo $? > {output_path}; [[ 2**3**2 -eq 512 ]]; echo $? >> {output_path}; (( n **= 4 )); echo $? $n >> {output_path}; (( 2 ** -1 )); echo $? >> {output_path}; (( 2 ** 200 )); echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0\n0\n0 16\n1\n1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_based_integer_constants() {
        let output_path = "target/rubash-arithmetic-command-bases-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "(( 2#101 - 5 )); echo $? > {output_path}; [[ 16#FF -eq 255 ]]; echo $? >> {output_path}; [[ 0x10 -eq 16 ]]; echo $? >> {output_path}; [[ 010 -eq 8 ]]; echo $? >> {output_path}; [[ 64#_ -eq 63 ]]; echo $? >> {output_path}; (( 8#9 )); echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "1\n0\n0\n0\n0\n1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_conditional_operator() {
        let output_path = "target/rubash-arithmetic-command-conditional-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=3; (( n > 2 ? n - 3 : 9 )); echo $? > {output_path}; (( n < 2 ? 0 : n + 1 )); echo $? >> {output_path}; [[ n==3?7:9 -eq 7 ]]; echo $? >> {output_path}; [[ n==4?7:9 -eq 9 ]]; echo $? >> {output_path}; [[ 0?1:0?2:3 -eq 3 ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n0\n0\n0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_drives_if_conditions() {
        let output_path = "target/rubash-arithmetic-if-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=1; if (( n )); then echo yes > {output_path}; else echo no > {output_path}; fi; if (( n - 1 )); then echo bad >> {output_path}; elif (( n + 1 )); then echo elif >> {output_path}; else echo bad >> {output_path}; fi; if (( n++ )); then echo $n >> {output_path}; fi"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\nelif\n2\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_drives_loop_conditions() {
        let output_path = "target/rubash-arithmetic-loop-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=0; while (( n < 3 )); do echo $n >> {output_path}; (( n++ )); done; until (( n == 5 )); do (( n++ )); done; echo $n >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n5\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_conditional_string_order_operators_are_not_redirects() {
        let output_path = "target/rubash-conditional-string-order-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "left=abc; right=def; [[ $left < $right ]]; echo $? > {output_path}; [[ $right > $left ]]; echo $? >> {output_path}; [[ $right < $left ]]; echo $? >> {output_path}; [[ $left > $right ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n0\n1\n1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_conditional_string_equality_uses_shell_patterns() {
        let output_path = "target/rubash-conditional-pattern-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "value=abcdef; pattern='a*'; [[ $value == a* ]]; echo $? > {output_path}; [[ $value = a?c* ]]; echo $? >> {output_path}; [[ $value == a[b-d]cdef ]]; echo $? >> {output_path}; [[ $value != z* ]]; echo $? >> {output_path}; [[ $value != a* ]]; echo $? >> {output_path}; [[ $value == $pattern ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0\n0\n0\n0\n1\n0\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_conditional_regex_match_sets_bash_rematch() {
        let output_path = "target/rubash-conditional-regex-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "value=abc123; pattern='([a-z]+)([0-9]+)'; [[ $value =~ $pattern ]]; echo $? ${{BASH_REMATCH[0]}} ${{BASH_REMATCH[1]}} ${{BASH_REMATCH[2]}} > {output_path}; [[ $value =~ z+ ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0 abc123 abc 123\n1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_conditional_logical_operators_stay_inside_expression() {
        let output_path = "target/rubash-conditional-logical-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "value=abc; empty=; [[ -n $value && -z $empty ]]; echo $? > {output_path}; [[ -n $empty || $value = abc ]]; echo $? >> {output_path}; [[ -n $empty || -z $value && $value = abc ]]; echo $? >> {output_path}; [[ ! -n $empty && $value = abc ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n0\n1\n0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_conditional_parentheses_group_logical_expressions() {
        let output_path = "target/rubash-conditional-parentheses-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "value=abc; empty=; [[ -n $value || -n $empty && -z $value ]]; echo $? > {output_path}; [[ ( -n $value || -n $empty ) && -z $value ]]; echo $? >> {output_path}; [[ ! ( -n $empty || -z $value ) ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_conditional_file_unary_checks_paths() {
        let output_path = "target/rubash-conditional-file-unary-output.txt";
        let file_path = "target/rubash-conditional-file-unary.txt";
        let dir_path = "target/rubash-conditional-file-unary-dir";
        let missing_path = "target/rubash-conditional-file-unary-missing";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(file_path);
        let _ = fs::remove_dir_all(dir_path);
        fs::write(file_path, "data").unwrap();
        fs::create_dir_all(dir_path).unwrap();
        let input = format!(
            "[[ -e {file_path} ]]; echo $? > {output_path}; [[ -f {file_path} ]]; echo $? >> {output_path}; [[ -d {dir_path} ]]; echo $? >> {output_path}; [[ -s {file_path} ]]; echo $? >> {output_path}; [[ -e {missing_path} ]]; echo $? >> {output_path}; [[ -d {file_path} ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0\n0\n0\n0\n1\n1\n"
        );
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(file_path);
        let _ = fs::remove_dir_all(dir_path);
    }

    #[test]
    fn test_conditional_negates_supported_expressions() {
        let output_path = "target/rubash-conditional-negation-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "value=abc; empty=; [[ ! -n $value ]]; echo $? > {output_path}; [[ ! -n $empty ]]; echo $? >> {output_path}; [[ ! 3 -gt 4 ]]; echo $? >> {output_path}; [[ ! $value = abc ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n0\n1\n");
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
