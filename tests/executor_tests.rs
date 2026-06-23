//! Executor Tests - TDD for Bash Command Executor
//!
//! Run with: cargo test --test executor_tests

use rubash::executor::{ExecuteError, Executor};
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn shell_test_path(path: &std::path::Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) && value.len() >= 3 && value.as_bytes()[1] == b':' {
        let drive = value.as_bytes()[0] as char;
        format!("/{}{}", drive.to_ascii_lowercase(), &value[2..])
    } else {
        value
    }
}

fn target_test_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(name)
}

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
    fn test_empty_command_does_not_reset_exit_status() {
        let output_path = "target/rubash-empty-command-status-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("false; ; echo $? > {output_path}");
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
    fn test_underscore_tracks_last_command_argument() {
        let output_path = "target/rubash-underscore-last-arg-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "printf '%s\\n' alpha beta > {output_path}; \
             printf '%s\\n' \"$_\" >> {output_path}; \
             :; printf '%s\\n' \"$_\" >> {output_path}; \
             _=temporary printf '%s\\n' final >> {output_path}; \
             printf '%s\\n' \"$_\" >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "alpha\nbeta\nbeta\n:\nfinal\nfinal\n"
        );
        let _ = fs::remove_file(output_path);
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
    fn test_pipeline_status_uses_last_command_by_default() {
        let output_path = "target/rubash-pipeline-status-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("false | true; echo $? > {output_path}");
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
    fn test_pipestatus_tracks_simple_and_pipeline_statuses() {
        let output_path = "target/rubash-pipestatus-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "echo $PIPESTATUS:${{PIPESTATUS[@]}}:${{#PIPESTATUS[@]}} > {output_path}; \
             false; echo $?:${{PIPESTATUS[@]}}:${{PIPESTATUS[0]}} >> {output_path}; \
             false | true | false; echo $? -- $PIPESTATUS -- ${{PIPESTATUS[@]}} -- ${{PIPESTATUS[0]}} - ${{PIPESTATUS[1]}} - ${{PIPESTATUS[2]}} >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0:0:1\n1:1:1\n1 -- 1 -- 1 0 1 -- 1 - 0 - 1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_pipefail_status_uses_rightmost_failing_command() {
        let output_path = "target/rubash-pipefail-status-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "set -o pipefail; false | true; echo $? > {output_path}; \
             true | false | true; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_pipefail_status_keeps_pipestatus_entries() {
        let output_path = "target/rubash-pipefail-pipestatus-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("set -o pipefail; false | true; echo $? -- ${{PIPESTATUS[@]}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1 -- 1 0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_pipefail_status_preserves_pipeline_output() {
        let output_path = "target/rubash-pipefail-output-status.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "set -o pipefail; printf 'a\\nb\\n' | grep z | wc -l > {output_path}; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_inverted_pipeline_uses_pipefail_status() {
        let output_path = "target/rubash-pipefail-invert-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("set -o pipefail; ! false | true; echo $? > {output_path}");
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

    #[test]
    fn test_printf_time_format_minus_two_uses_shell_start_time() {
        let output_path = "target/rubash-printf-time-shell-start-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("TZ=UTC printf '%(%s)T' -2 > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let shell_start = executor
            .get_env("__RUBASH_SHELL_START_EPOCH")
            .unwrap()
            .to_string();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), shell_start);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_printf_invalid_time_format_warns_and_outputs_raw_format() {
        let output_path = "target/rubash-printf-invalid-time-output.txt";
        let status_path = "target/rubash-printf-invalid-time-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!("printf '%(abde)Z\\n' -1 > {output_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "%(abde)Z\n");
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_epochseconds_expands_to_current_epoch_time() {
        let output_path = "target/rubash-epochseconds-output.txt";
        let _ = fs::remove_file(output_path);
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let input =
            format!("printf '%s\\n' \"$EPOCHSECONDS\" \"${{EPOCHSECONDS}}\" > {output_path}");
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
        let values = fs::read_to_string(output_path)
            .unwrap()
            .lines()
            .map(|line| line.parse::<i64>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(|value| (before..=after).contains(value)));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_epochrealtime_expands_with_microseconds() {
        let output_path = "target/rubash-epochrealtime-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("printf '%s\\n' \"$EPOCHREALTIME\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        let value = output.trim_end();
        let (seconds, micros) = value.split_once('.').expect("epoch realtime decimal");
        assert!(seconds.parse::<i64>().is_ok());
        assert_eq!(micros.len(), 6);
        assert!(micros.chars().all(|ch| ch.is_ascii_digit()));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_seconds_assignment_resets_dynamic_counter() {
        let output_path = "target/rubash-seconds-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("SECONDS=7; printf '%s\\n' \"$SECONDS\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let seconds: i64 = fs::read_to_string(output_path)
            .unwrap()
            .trim_end()
            .parse()
            .unwrap();
        assert!((7..=8).contains(&seconds));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_argv0_assignment_updates_zero_parameter() {
        let output_path = "target/rubash-bash-argv0-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("BASH_ARGV0=hello; printf '%s:%s\\n' \"$0\" \"$BASH_ARGV0\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "hello:hello\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_argv0_defaults_to_shell_name() {
        let output_path = "target/rubash-bash-argv0-default-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("printf '%s:%s\\n' \"$0\" \"$BASH_ARGV0\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "rubash:rubash\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_argv0_reflects_script_name() {
        let output_path = "target/rubash-bash-argv0-script-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("printf '%s:%s\\n' \"$0\" \"$BASH_ARGV0\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("__RUBASH_SCRIPT_NAME", "./script.sh");

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "./script.sh:./script.sh\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_dynamic_bash_parameter_lengths_use_current_values() {
        let output_path = "target/rubash-dynamic-param-length-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("printf '%s:%s:%s\\n' \"${{#BASH_ARGV0}}\" \"${{#BASHPID}}\" \"${{#BASH_SUBSHELL}}\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("__RUBASH_SCRIPT_NAME", "./script.sh");

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            format!("11:{}:1\n", std::process::id().to_string().chars().count())
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_argv0_assignment_inside_function_updates_zero_parameter() {
        let output_path = "target/rubash-bash-argv0-function-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "setarg0() {{ BASH_ARGV0=\"$1\"; }}; setarg0 arg0; printf '%s:%s\\n' \"$0\" \"$BASH_ARGV0\" > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "arg0:arg0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_random_expands_to_15_bit_values_and_advances() {
        let output_path = "target/rubash-random-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("printf '%s\\n' \"$RANDOM\" \"$RANDOM\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let values = fs::read_to_string(output_path)
            .unwrap()
            .lines()
            .map(|line| line.parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(|value| *value <= 32767));
        assert_ne!(values[0], values[1]);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_random_assignment_reseeds_sequence() {
        let output_path = "target/rubash-random-seed-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "RANDOM=42; first=$RANDOM; RANDOM=42; second=$RANDOM; printf '%s:%s\\n' \"$first\" \"$second\" > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        let (first, second) = output.trim_end().split_once(':').unwrap();
        assert_eq!(first, second);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_random_expands_inside_arithmetic_expressions() {
        let output_path = "target/rubash-random-arithmetic-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "RANDOM=42; first=$((RANDOM)); RANDOM=42; second=$((RANDOM)); printf '%s:%s\\n' \"$first\" \"$second\" > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        let (first, second) = output.trim_end().split_once(':').unwrap();
        assert_eq!(first, second);
        assert!(first.parse::<u32>().unwrap() <= 32767);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_random_advances_inside_arithmetic_command() {
        let output_path = "target/rubash-random-arithmetic-command-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "RANDOM=42; (( first=RANDOM, second=RANDOM )); printf '%s:%s\\n' \"$first\" \"$second\" > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        let (first, second) = output.trim_end().split_once(':').unwrap();
        assert_ne!(first, second);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bashpid_expands_to_current_shell_pid() {
        let output_path = "target/rubash-bashpid-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("printf '%s:%s\\n' \"$BASHPID\" \"$$\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        let (bashpid, shell_pid) = output.trim_end().split_once(':').unwrap();
        assert_eq!(bashpid, shell_pid);
        assert_eq!(bashpid.parse::<u32>().unwrap(), std::process::id());
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bashpid_assignment_does_not_override_dynamic_value() {
        let output_path = "target/rubash-bashpid-assignment-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("BASHPID=1; printf '%s\\n' \"$BASHPID\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap().trim_end(),
            std::process::id().to_string()
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bashpid_changes_in_command_substitution() {
        let output_path = "target/rubash-bashpid-command-substitution-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "printf '%s:%s:%s\\n' \"$$\" \"$BASHPID\" \"$(echo $BASHPID)\" > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        let parts: Vec<_> = output.trim_end().split(':').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], parts[1]);
        assert_ne!(parts[0], parts[2]);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_last_background_pid_parameter_tracks_background_command() {
        let output_path = "target/rubash-last-background-pid-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "printf '<%s>\\n' \"$!\" > {output_path}; false & printf 'status:%s bang:%s\\n' \"$?\" \"$!\" >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            format!("<>\nstatus:0 bang:{}\n", std::process::id())
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_subshell_tracks_command_substitution_depth() {
        let output_path = "target/rubash-bash-subshell-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "printf '%s:%s:%s\\n' \"$BASH_SUBSHELL\" \"$(echo $BASH_SUBSHELL)\" \"$BASH_SUBSHELL\" > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0:1:0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_subshell_tracks_parenthesized_subshell() {
        let output_path = "target/rubash-bash-subshell-parenthesized-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "( printf '%s\\n' \"$BASH_SUBSHELL\" > {output_path} ); printf '%s\\n' \"$BASH_SUBSHELL\" >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_subshell_assignment_does_not_override_dynamic_value() {
        let output_path = "target/rubash-bash-subshell-assignment-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("BASH_SUBSHELL=9; printf '%s\\n' \"$BASH_SUBSHELL\" > {output_path}");
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
    fn test_bash_command_expands_to_current_command() {
        let output_path = "target/rubash-bash-command-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("echo $BASH_COMMAND > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            format!("echo $BASH_COMMAND > {output_path}\n")
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_command_assignment_does_not_override_dynamic_value() {
        let output_path = "target/rubash-bash-command-assignment-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("BASH_COMMAND=ignored; echo $BASH_COMMAND > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            format!("echo $BASH_COMMAND > {output_path}\n")
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_version_and_versinfo_are_initialized() {
        let output_path = "target/rubash-bash-version-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "printf '%s\\n%s\\n%s\\n%s\\n%s:%s:%s\\n' \"$BASH\" \"$BASH_VERSION\" \"${{BASH_VERSINFO[@]}}\" \"$HOSTNAME\" \"$HOSTTYPE\" \"$OSTYPE\" \"$MACHTYPE\" > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert!(!lines[0].is_empty());
        assert!(lines[1].starts_with(env!("CARGO_PKG_VERSION")));
        assert!(lines[1].ends_with("(1)-release"));
        let version_words = env!("CARGO_PKG_VERSION").replace('.', " ");
        assert!(lines[2].starts_with(&format!("{version_words} 1 release ")));
        assert_eq!(lines[2].split_whitespace().count(), 6);
        assert!(!lines[3].is_empty());
        assert_eq!(lines[4].split(':').count(), 3);
        assert!(!lines[4].contains("::"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_versinfo_assignment_reports_readonly() {
        let output_path = "target/rubash-bash-versinfo-readonly-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("BASH_VERSINFO[0]=9; echo $? ${{BASH_VERSINFO[0]}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1 0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_call_stack_arrays_are_initialized() {
        let output_path = "target/rubash-bash-call-stack-arrays-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("declare -p BASH_ARGC BASH_ARGV BASH_LINENO BASH_SOURCE > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("__RUBASH_SCRIPT_NAME", "./call-stack.tests");

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("declare -a BASH_ARGC=()\n"));
        assert!(output.contains("declare -a BASH_ARGV=()\n"));
        assert!(output.contains("declare -a BASH_LINENO=([0]=\"0\")\n"));
        assert!(output.contains("declare -a BASH_SOURCE=([0]=\"./call-stack.tests\")\n"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_call_stack_arrays_ignore_assignment() {
        let output_path = "target/rubash-bash-call-stack-noassign-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "BASH_ARGC=(xxx); echo argc:$?:${{#BASH_ARGC[@]}} > {output_path}; \
             declare BASH_ARGV[1]=foo; echo argv:$?:${{#BASH_ARGV[@]}} >> {output_path}; \
             BASH_SOURCE[0]=other; echo source:$?:${{BASH_SOURCE[0]}} >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("__RUBASH_SCRIPT_NAME", "./call-stack.tests");

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "argc:0:0\nargv:0:0\nsource:0:./call-stack.tests\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_call_stack_arrays_cannot_be_unset() {
        let output_path = "target/rubash-bash-call-stack-unset-output.txt";
        let error_path = "target/rubash-bash-call-stack-unset-error.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
        let input = format!(
            "unset -v BASH_LINENO BASH_SOURCE 2> {error_path}; \
             echo status:$? > {output_path}; \
             echo values:${{BASH_LINENO[0]}}:${{BASH_SOURCE[0]}} >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("__RUBASH_SCRIPT_NAME", "./call-stack-unset.case");

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "status:1\nvalues:0:./call-stack-unset.case\n"
        );
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("unset: BASH_LINENO: cannot unset"));
        assert!(error.contains("unset: BASH_SOURCE: cannot unset"));
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_uid_and_euid_are_readonly_nonzero_ids() {
        let output_path = "target/rubash-uid-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "echo $UID:$EUID > {output_path}; if (( UID == 0 )); then echo root >> {output_path}; else echo user >> {output_path}; fi; test -R UID; echo readonly:$? >> {output_path}; UID=0; echo assign:$?:$UID >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        let (uid, euid) = lines[0].split_once(':').unwrap();
        assert!(!uid.is_empty());
        assert!(!euid.is_empty());
        assert_eq!(lines[1], if uid == "0" { "root" } else { "user" });
        assert_eq!(lines[2], "readonly:0");
        assert_eq!(lines[3], format!("assign:1:{uid}"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_ppid_is_readonly_numeric_id() {
        let output_path = "target/rubash-ppid-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "echo $PPID > {output_path}; \
             test -R PPID; echo readonly:$? >> {output_path}; \
             PPID=1; echo assign:$?:$PPID >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert!(lines[0].chars().all(|ch| ch.is_ascii_digit()));
        assert_eq!(lines[1], "readonly:0");
        assert_eq!(lines[2], format!("assign:1:{}", lines[0]));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_funcname_expands_inside_function() {
        let output_path = "target/rubash-funcname-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("show_name() {{ printf '%s\\n' \"$FUNCNAME\" > {output_path}; }}; show_name");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "show_name\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_funcname_tracks_nested_function_stack() {
        let output_path = "target/rubash-funcname-stack-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "outer() {{ inner; }}; \
             inner() {{ printf '%s|%s|%s|%s|%s\\n' \"$FUNCNAME\" \"${{FUNCNAME[0]}}\" \"${{FUNCNAME[1]}}\" \"${{FUNCNAME[@]}}\" \"${{#FUNCNAME[@]}}\" > {output_path}; }}; \
             outer"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "inner|inner|outer|inner outer|2\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_lineno_and_source_track_function_stack() {
        let output_path = "target/rubash-bash-lineno-source-stack-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "outer() {{ inner; }}; \
             inner() {{ printf '%s|%s|%s|%s|%s\\n' \"${{BASH_LINENO[0]}}\" \"${{BASH_LINENO[1]}}\" \"${{BASH_SOURCE[0]}}\" \"${{BASH_SOURCE[1]}}\" \"${{#BASH_SOURCE[@]}}\" > {output_path}; }}; \
             outer"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("__RUBASH_SCRIPT_NAME", "./stack-source.tests");

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "1|1|./stack-source.tests|./stack-source.tests|3\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_argc_and_argv_track_function_arguments() {
        let output_path = "target/rubash-bash-argc-argv-stack-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "outer() {{ inner c; }}; \
             inner() {{ printf '%s|%s|%s|%s|%s\\n' \"${{BASH_ARGC[0]}}\" \"${{BASH_ARGC[1]}}\" \"${{BASH_ARGV[0]}}\" \"${{BASH_ARGV[1]}}\" \"${{BASH_ARGV[@]}}\" > {output_path}; }}; \
             outer a b"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1|2|c|b|c b a\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_funcname_assignment_does_not_change_stack() {
        let output_path = "target/rubash-funcname-noassign-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "show_name() {{ FUNCNAME[0]=bad; declare FUNCNAME=also_bad; printf '%s:%s:%s\\n' \"$?\" \"$FUNCNAME\" \"${{FUNCNAME[0]}}\" > {output_path}; }}; show_name"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0:show_name:show_name\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_funcname_assignment_does_not_override_dynamic_value() {
        let output_path = "target/rubash-funcname-assignment-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("FUNCNAME=42; printf '<%s>\\n' \"$FUNCNAME\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "<>\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_groups_expands_as_dynamic_array() {
        let output_path = "target/rubash-groups-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("printf '%s:%s:%s\\n' \"$GROUPS\" \"${{GROUPS[0]}}\" \"${{#GROUPS[@]}}\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0:0:1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_groups_assignment_does_not_override_dynamic_array() {
        let output_path = "target/rubash-groups-assignment-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "GROUPS[0]=-1; status=$?; printf '%s:%s\\n' \"$status\" \"${{GROUPS[0]}}\" > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0:0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_lineno_expands_to_current_command_line() {
        let output_path = "target/rubash-lineno-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("printf '%s:%s\\n' \"$LINENO\" \"$((LINENO))\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1:1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_lineno_assignment_does_not_override_dynamic_value() {
        let output_path = "target/rubash-lineno-assignment-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("LINENO=99; printf '%s\\n' \"$LINENO\" > {output_path}");
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
    fn test_pipeline_feeds_external_stage_stdin() {
        let output_path = target_test_path("rubash-pipeline-external-output.txt");
        #[cfg(windows)]
        let script_path = target_test_path("rubash-pipeline-filter.cmd");
        #[cfg(not(windows))]
        let script_path = target_test_path("rubash-pipeline-filter.sh");
        let shell_output_path = shell_test_path(&output_path);
        let shell_script_path = shell_test_path(&script_path);
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_file(&script_path);
        #[cfg(windows)]
        fs::write(
            &script_path,
            "@echo off\r\n\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoProfile -Command \"$input | Where-Object { $_ -eq 'b' } | ForEach-Object { 'external:' + $_ }\"\r\n",
        )
        .unwrap();
        #[cfg(not(windows))]
        fs::write(
            &script_path,
            "#!/bin/sh\nwhile IFS= read -r line; do\n  if [ \"$line\" = b ]; then\n    printf 'external:%s\\n' \"$line\"\n  fi\ndone\n",
        )
        .unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&script_path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script_path, permissions).unwrap();
        }
        let input = format!("printf 'a\\nb\\n' | {shell_script_path} > {shell_output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
        assert!(ast.commands[0].pipe.is_some());
        assert_eq!(ast.commands[1].words, [shell_script_path.as_str()]);
        assert!(ast.commands[1].redirect_out.is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(&output_path)
                .unwrap()
                .replace("\r\n", "\n"),
            "external:b\n"
        );
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(script_path);
    }

    #[test]
    fn test_external_command_appends_stdout() {
        let bin_dir = "target/rubash-external-append-bin";
        let script_path = format!("{bin_dir}/emit");
        let output_path = "target/rubash-external-append-output.txt";
        let _ = fs::remove_dir_all(bin_dir);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&script_path, "echo external-append\n").unwrap();
        let input = format!("emit > {output_path}; emit >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "external-append\nexternal-append\n"
        );
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir_all(bin_dir);
    }

    #[test]
    fn test_external_command_appends_stderr() {
        let bin_dir = "target/rubash-external-stderr-append-bin";
        let script_path = format!("{bin_dir}/emiterr");
        let output_path = "target/rubash-external-stderr-append-output.txt";
        let _ = fs::remove_dir_all(bin_dir);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&script_path, "echo external-error >&2\n").unwrap();
        let input = format!("emiterr 2> {output_path}; emiterr 2>> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "external-error\nexternal-error\n"
        );
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir_all(bin_dir);
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
    fn test_mapfile_without_t_preserves_record_newlines() {
        let input = "mapfile arr <<< $'alpha\\nbeta'";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            executor.get_env("arr"),
            Some("\x1d([0]=$'alpha\\n' [1]=$'beta\\n')")
        );
    }
    #[test]
    fn test_mapfile_t_preserves_trailing_empty_line() {
        let output_path = "target/rubash-mapfile-trailing-empty-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "mapfile -t arr <<< $'alpha\\n'; echo ${{#arr[@]}} ${{#arr[1]}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "2 0\n");
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
    fn test_mapfile_n_zero_reads_all_lines() {
        let output_path = "target/rubash-mapfile-n-zero-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "mapfile -n 0 -t arr <<< $'alpha\\nbeta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
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
    fn test_mapfile_callback_runs_at_quantum() {
        let output_path = "target/rubash-mapfile-callback-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "cb() {{ echo \"$1:$2\" >> {output_path}; }}; mapfile -C cb -c 2 -t arr <<< $'alpha\\nbeta\\ngamma'; echo ${{#arr[@]}} ${{arr[@]}} >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "1:beta\n3 alpha beta gamma\n"
        );
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
    fn test_bash_cmds_reflects_hash_table() {
        let output_path = "target/rubash-bash-cmds-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "hash -r; \
             hash -p /usr/sbin/foo foo; \
             BASH_CMDS[bar]=/usr/bin/bar; \
             printf '%s\\n' \"${{!BASH_CMDS[@]}}\" \"${{BASH_CMDS[@]}}\" \"${{BASH_CMDS[foo]}}\" > {output_path}; \
             hash -t bar >> {output_path}; \
             unset 'BASH_CMDS[foo]'; hash -t foo 2>/dev/null; echo $? >> {output_path}; \
             declare -p BASH_CMDS >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "bar foo\n/usr/bin/bar /usr/sbin/foo\n/usr/sbin/foo\n/usr/bin/bar\n1\ndeclare -A BASH_CMDS=([bar]=\"/usr/bin/bar\" )\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_aliases_reflects_alias_table() {
        let output_path = "target/rubash-bash-aliases-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "alias foo=/usr/sbin/foo; \
             BASH_ALIASES[bar]=/usr/bin/bar; \
             printf '%s\\n' \"${{!BASH_ALIASES[@]}}\" \"${{BASH_ALIASES[@]}}\" \"${{BASH_ALIASES[foo]}}\" > {output_path}; \
             unset 'BASH_ALIASES[foo]'; \
             alias foo 2>/dev/null; echo $? >> {output_path}; \
             declare -p BASH_ALIASES >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "bar foo\n/usr/bin/bar /usr/sbin/foo\n/usr/sbin/foo\n1\ndeclare -A BASH_ALIASES=([bar]=\"/usr/bin/bar\" )\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bash_aliases_preserves_values_with_spaces() {
        let output_path = "target/rubash-bash-aliases-spaces-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "alias qux='/usr/local/bin/qux -l'; \
             BASH_ALIASES[blat]='cd /blat ; echo $PWD'; \
             printf '%s\\n' \"${{BASH_ALIASES[qux]}}\" \"${{BASH_ALIASES[blat]}}\" > {output_path}; \
             declare -p BASH_ALIASES >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "/usr/local/bin/qux -l\ncd /blat ; echo $PWD\ndeclare -A BASH_ALIASES=([blat]=\"cd /blat ; echo \\$PWD\" [qux]=\"/usr/local/bin/qux -l\" )\n"
        );
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
    fn test_declare_p_without_names_lists_variables() {
        let output_path = "target/rubash-declare-p-all-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("RUBASH_DECLARE_ALL=value; declare -p > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("declare -- RUBASH_DECLARE_ALL=\"value\"\n"));
        assert!(!output.contains("__RUBASH_EXPORTED_VARS"));
        std::env::remove_var("RUBASH_DECLARE_ALL");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_declare_without_names_lists_variables() {
        let output_path = "target/rubash-declare-all-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("RUBASH_DECLARE_BARE=value; declare > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert!(fs::read_to_string(output_path)
            .unwrap()
            .contains("declare -- RUBASH_DECLARE_BARE=\"value\"\n"));
        std::env::remove_var("RUBASH_DECLARE_BARE");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_declare_attribute_filters_without_names() {
        let output_path = "target/rubash-declare-attr-filter-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "plain=value; declare -i RUBASH_DECLARE_INT=7; \
             declare -u RUBASH_DECLARE_UP=abc; readonly RUBASH_DECLARE_RO=1; \
             declare -i > {output_path}; declare -u >> {output_path}; declare -r >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("declare -i RUBASH_DECLARE_INT=\"7\"\n"));
        assert!(output.contains("declare -u RUBASH_DECLARE_UP=\"ABC\"\n"));
        assert!(output.contains("declare -r RUBASH_DECLARE_RO=\"1\"\n"));
        assert!(!output.contains("plain=\"value\""));
        std::env::remove_var("RUBASH_DECLARE_INT");
        std::env::remove_var("RUBASH_DECLARE_UP");
        std::env::remove_var("RUBASH_DECLARE_RO");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_declare_marks_inherited_environment_as_exported() {
        let output_path = "target/rubash-declare-inherited-export-output.txt";
        let _ = fs::remove_file(output_path);
        let old_value = std::env::var("RUBASH_INHERITED_EXPORT").ok();
        std::env::set_var("RUBASH_INHERITED_EXPORT", "from-env");
        let input = format!(
            "RUBASH_NOT_EXPORTED=local; \
             declare -p RUBASH_INHERITED_EXPORT > {output_path}; \
             declare -x >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        match old_value {
            Some(value) => std::env::set_var("RUBASH_INHERITED_EXPORT", value),
            None => std::env::remove_var("RUBASH_INHERITED_EXPORT"),
        }
        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("declare -x RUBASH_INHERITED_EXPORT=\"from-env\"\n"));
        assert!(!output.contains("RUBASH_NOT_EXPORTED"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_declare_r_marks_readonly_variable() {
        let output_path = "target/rubash-declare-readonly-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("declare -r RO=1; declare -p RO > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "declare -r RO=\"1\"\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_declare_rx_marks_exported_readonly_variable() {
        let output_path = "target/rubash-declare-readonly-export-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("declare -rx REX=2; declare -p REX > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "declare -rx REX=\"2\"\n"
        );
        std::env::remove_var("REX");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_declare_redirects_stderr() {
        let error_path = "target/rubash-declare-stderr-output.txt";
        let status_path = "target/rubash-declare-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("declare -Z 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("rubash: declare: -Z: unsupported option"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_declare_appends_stderr() {
        let error_path = "target/rubash-declare-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("declare -Z 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("rubash: declare: -Z: unsupported option"));
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_builtin_declare_assigns_variable() {
        let output_path = "target/rubash-builtin-declare-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin declare RUBASH_BUILTIN_DECLARE=value; echo $RUBASH_BUILTIN_DECLARE > {output_path}");
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
    fn test_builtin_typeset_assigns_variable() {
        let output_path = "target/rubash-builtin-typeset-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin typeset RUBASH_BUILTIN_TYPESET=value; echo $RUBASH_BUILTIN_TYPESET > {output_path}");
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
    fn test_export_p_omits_unexported_shell_variables() {
        let output_path = "target/rubash-export-p-unexported-output.txt";
        let _ = fs::remove_file(output_path);
        let old_value = std::env::var("RUBASH_EXPORT_INHERITED").ok();
        std::env::set_var("RUBASH_EXPORT_INHERITED", "from-env");
        let input = format!(
            "RUBASH_EXPORT_LOCAL=local; export RUBASH_EXPORT_MARKED=value; export -p > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        match old_value {
            Some(value) => std::env::set_var("RUBASH_EXPORT_INHERITED", value),
            None => std::env::remove_var("RUBASH_EXPORT_INHERITED"),
        }
        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("declare -x RUBASH_EXPORT_INHERITED=\"from-env\"\n"));
        assert!(output.contains("declare -x RUBASH_EXPORT_MARKED=\"value\"\n"));
        assert!(!output.contains("RUBASH_EXPORT_LOCAL"));
        std::env::remove_var("RUBASH_EXPORT_MARKED");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_export_n_clears_export_attribute_without_unsetting_variable() {
        let output_path = "target/rubash-export-n-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "RUBASH_EXPORT_CLEAR=value; export RUBASH_EXPORT_CLEAR; export -n RUBASH_EXPORT_CLEAR; \
             printf '<%s>\\n' \"$RUBASH_EXPORT_CLEAR\" > {output_path}; \
             export -p >> {output_path}; printf '%s\\n' --- >> {output_path}; \
             declare -p RUBASH_EXPORT_CLEAR >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("<value>\n"));
        assert!(!output.contains("declare -x RUBASH_EXPORT_CLEAR="));
        assert!(output.contains("---\ndeclare -- RUBASH_EXPORT_CLEAR=\"value\"\n"));
        std::env::remove_var("RUBASH_EXPORT_CLEAR");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_child_environment_contains_only_exported_variables() {
        let output_path = target_test_path("rubash-child-export-env-output.txt");
        let _ = fs::remove_file(&output_path);
        let shell_output_path = shell_test_path(&output_path);
        let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
        let input = format!(
            "RUBASH_CHILD_LOCAL=local; export RUBASH_CHILD_EXPORTED=exported; \
             {rubash} -c 'printf \"%s/%s\\n\" \"${{RUBASH_CHILD_LOCAL-unset}}\" \"$RUBASH_CHILD_EXPORTED\"' > {shell_output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(&output_path).unwrap(),
            "unset/exported\n"
        );
        std::env::remove_var("RUBASH_CHILD_LOCAL");
        std::env::remove_var("RUBASH_CHILD_EXPORTED");
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_temporary_assignment_reaches_external_command_environment() {
        let output_path = target_test_path("rubash-temp-assignment-env-output.txt");
        let _ = fs::remove_file(&output_path);
        let shell_output_path = shell_test_path(&output_path);
        let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
        let input = format!(
            "RUBASH_TEMP_ENV=command {rubash} -c 'printf \"%s\\n\" \"$RUBASH_TEMP_ENV\"' > {shell_output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "command\n");
        std::env::remove_var("RUBASH_TEMP_ENV");
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_child_environment_includes_exported_pwd() {
        let output_path = target_test_path("rubash-child-pwd-env-output.txt");
        #[cfg(windows)]
        let script_path = target_test_path("rubash-child-pwd-env.cmd");
        #[cfg(not(windows))]
        let script_path = target_test_path("rubash-child-pwd-env.sh");
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_file(&script_path);
        let shell_output_path = shell_test_path(&output_path);
        let shell_script_path = shell_test_path(&script_path);
        #[cfg(windows)]
        fs::write(
            &script_path,
            "@echo off\r\nif \"%PWD%\"==\"\" (echo unset) else echo %PWD%\r\n",
        )
        .unwrap();
        #[cfg(not(windows))]
        fs::write(&script_path, "#!/bin/sh\nprintf '%s\\n' \"${PWD-unset}\"\n").unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&script_path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script_path, permissions).unwrap();
        }
        let input = format!("{shell_script_path} > {shell_output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(&output_path)
            .unwrap()
            .replace("\r\n", "\n");
        assert_ne!(output, "unset\n");
        assert!(!output.trim().is_empty());
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_file(&script_path);
    }

    #[test]
    fn test_shell_level_increments_inherited_environment() {
        let output_path = target_test_path("rubash-shlvl-output.txt");
        let _ = fs::remove_file(&output_path);
        let shell_output_path = shell_test_path(&output_path);
        let original_shlvl = std::env::var("SHLVL").ok();
        std::env::set_var("SHLVL", "7");
        let input = format!("printf '%s\\n' \"$SHLVL\" > {shell_output_path}; declare -p SHLVL >> {shell_output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        match original_shlvl {
            Some(value) => std::env::set_var("SHLVL", value),
            None => std::env::remove_var("SHLVL"),
        }
        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(&output_path).unwrap(),
            "8\ndeclare -x SHLVL=\"8\"\n"
        );
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_child_rubash_increments_shell_level() {
        let output_path = target_test_path("rubash-child-shlvl-output.txt");
        let _ = fs::remove_file(&output_path);
        let shell_output_path = shell_test_path(&output_path);
        let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
        let input =
            format!("SHLVL=12 {rubash} -c 'printf \"%s\\n\" \"$SHLVL\"' > {shell_output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "13\n");
        let _ = fs::remove_file(&output_path);
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
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("declare -r RUBASH_READONLY_REDIR=\"value\"\n"));
        assert!(output.contains("declare -r SHELLOPTS=\""));
        std::env::remove_var("RUBASH_READONLY_REDIR");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_export_assigns_variable() {
        let output_path = "target/rubash-builtin-export-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin export RUBASH_BUILTIN_EXPORT=value; echo $RUBASH_BUILTIN_EXPORT > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "value\n");
        std::env::remove_var("RUBASH_BUILTIN_EXPORT");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_readonly_assigns_variable() {
        let output_path = "target/rubash-builtin-readonly-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin readonly RUBASH_BUILTIN_READONLY=value; echo $RUBASH_BUILTIN_READONLY > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "value\n");
        std::env::remove_var("RUBASH_BUILTIN_READONLY");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_export_redirects_stderr() {
        let error_path = "target/rubash-export-stderr-output.txt";
        let status_path = "target/rubash-export-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("export -Z 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("rubash: export: -Z: invalid option"));
        assert!(error.contains("export: usage:"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_export_appends_stderr() {
        let error_path = "target/rubash-export-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("export -Z 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 2);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("rubash: export: -Z: invalid option"));
        assert!(error.contains("export: usage:"));
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_readonly_redirects_stderr() {
        let error_path = "target/rubash-readonly-stderr-output.txt";
        let status_path = "target/rubash-readonly-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("readonly -Z 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("readonly: -Z: invalid option"));
        assert!(error.contains("readonly: usage:"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_readonly_appends_stderr() {
        let error_path = "target/rubash-readonly-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("readonly -Z 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 2);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("readonly: -Z: invalid option"));
        assert!(error.contains("readonly: usage:"));
        let _ = fs::remove_file(error_path);
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
    fn test_pwd_redirects_stderr() {
        let error_path = "target/rubash-pwd-stderr-output.txt";
        let status_path = "target/rubash-pwd-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);

        let input = format!("pwd -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("rubash: pwd: -x: invalid option"));
        assert!(error.contains("pwd: usage: pwd [-LP]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_pwd_appends_stderr() {
        let error_path = "target/rubash-pwd-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();

        let input = format!("pwd -x 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 2);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("rubash: pwd: -x: invalid option"));
        assert!(error.contains("pwd: usage: pwd [-LP]"));
        let _ = fs::remove_file(error_path);
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
    fn test_hash_empty_table_reports_success() {
        let error_path = "target/rubash-hash-empty-stderr-output.txt";
        let status_path = "target/rubash-hash-empty-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("hash 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        assert!(fs::read_to_string(error_path)
            .unwrap()
            .contains("hash: hash table empty"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_hash_p_requires_pathname_argument() {
        let error_path = "target/rubash-hash-p-missing-stderr-output.txt";
        let status_path = "target/rubash-hash-p-missing-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("hash -p 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("hash: -p: option requires an argument"));
        assert!(error.contains("hash: usage:"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_hash_p_without_name_prints_table() {
        let output_path = "target/rubash-hash-p-without-name-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("hash -p /tmp/rubash-cat cat; hash -p /tmp/ignored > {output_path}; echo $? >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("hits\tcommand\n"));
        assert!(output.contains("/tmp/rubash-cat\n"));
        assert!(output.ends_with("0\n"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_hash_redirects_stderr() {
        let error_path = "target/rubash-hash-stderr-output.txt";
        let status_path = "target/rubash-hash-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("hash -t no_such_command 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("hash: no_such_command: not found"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_hash_appends_stderr() {
        let error_path = "target/rubash-hash-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("hash -t no_such_command 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("hash: no_such_command: not found"));
        let _ = fs::remove_file(error_path);
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
    fn test_builtin_shopt_updates_shell_option_state() {
        let output_path = "target/rubash-builtin-shopt-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("builtin shopt -s nullglob; shopt -q nullglob; echo $? > {output_path}");
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
    fn test_shopt_redirects_stderr() {
        let error_path = "target/rubash-shopt-stderr-output.txt";
        let status_path = "target/rubash-shopt-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("shopt -Z 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("shopt: -Z: invalid option"));
        assert!(error.contains("shopt: usage:"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_shopt_appends_stderr() {
        let error_path = "target/rubash-shopt-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("shopt -Z 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 2);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("shopt: -Z: invalid option"));
        assert!(error.contains("shopt: usage:"));
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_cdable_vars_uses_variable_as_directory() {
        let original_dir = std::env::current_dir().unwrap();
        let original_pwd = std::env::var("PWD").ok();
        let original_oldpwd = std::env::var("OLDPWD").ok();
        let root = original_dir.join("target/rubash-cdable-vars");
        let dest_dir = root.join("dest");
        let output_path = root.join("output.txt");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&dest_dir).unwrap();

        let dest_display = shell_test_path(&dest_dir);
        let output_display = output_path.to_string_lossy().replace('\\', "/");
        let input = format!(
            "shopt -s cdable_vars; dest='{dest_display}'; cd dest > {output_display}; echo $PWD >> {output_display}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);
        let _ = std::env::set_current_dir(&original_dir);
        match original_pwd {
            Some(value) => std::env::set_var("PWD", value),
            None => std::env::remove_var("PWD"),
        }
        match original_oldpwd {
            Some(value) => std::env::set_var("OLDPWD", value),
            None => std::env::remove_var("OLDPWD"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(&output_path).unwrap(),
            format!("{dest_display}\n{dest_display}\n")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_builtin_cd_invokes_cd_builtin() {
        let original_dir = std::env::current_dir().unwrap();
        let original_pwd = std::env::var("PWD").ok();
        let original_oldpwd = std::env::var("OLDPWD").ok();
        let root = original_dir.join("target/rubash-builtin-cd");
        let dest_dir = root.join("dest");
        let output_path = root.join("output.txt");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&dest_dir).unwrap();

        let dest_display = shell_test_path(&dest_dir);
        let output_display = output_path.to_string_lossy().replace('\\', "/");
        let input = format!(
            "function cd {{ echo function-cd; }}; builtin cd {dest_display}; pwd > {output_display}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);
        let _ = std::env::set_current_dir(&original_dir);
        match original_pwd {
            Some(value) => std::env::set_var("PWD", value),
            None => std::env::remove_var("PWD"),
        }
        match original_oldpwd {
            Some(value) => std::env::set_var("OLDPWD", value),
            None => std::env::remove_var("OLDPWD"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(&output_path).unwrap(),
            format!("{dest_display}\n")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_builtin_set_updates_shell_options() {
        let output_path = "target/rubash-builtin-set-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "builtin set -u; echo $- > {output_path}; [[ -o nounset ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let lines: Vec<String> = fs::read_to_string(output_path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert!(lines[0].contains('u'));
        assert_eq!(lines[1], "0");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_set_replaces_positional_parameters() {
        let output_path = "target/rubash-builtin-set-positionals-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin set -- alpha beta; echo $# $1 $2 > {output_path}");
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
    fn test_cd_redirects_stderr() {
        let error_path = "target/rubash-cd-stderr-output.txt";
        let status_path = "target/rubash-cd-stderr-status.txt";
        let missing_dir = "target/rubash-cd-no-such-dir";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);

        let input = format!("cd {missing_dir} 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("rubash: cd:"));
        assert!(error.contains("rubash-cd-no-such-dir"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_cd_appends_stderr() {
        let error_path = "target/rubash-cd-stderr-append-output.txt";
        let missing_dir = "target/rubash-cd-no-such-dir";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();

        let input = format!("cd {missing_dir} 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("rubash: cd:"));
        assert!(error.contains("rubash-cd-no-such-dir"));
        let _ = fs::remove_file(error_path);
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
    fn test_umask_symbolic_modes_update_mask() {
        let output_path = "target/rubash-umask-symbolic-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "umask 022; umask u=rwx,g=rx,o=; umask > {output_path}; \
             umask -S >> {output_path}; umask a+r; umask >> {output_path}; \
             umask -S >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0027\nu=rwx,g=rx,o=\n0023\nu=rwx,g=rx,o=r\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_umask_symbolic_modes_copy_permissions() {
        let output_path = "target/rubash-umask-symbolic-copy-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "umask 022; umask g+u,o+rwx-u; umask -S > {output_path}; \
             umask 022; umask o=u; umask -p -S >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "u=rwx,g=rwx,o=\numask -S u=rwx,g=rx,o=rwx\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_umask_redirects_stderr() {
        let error_path = "target/rubash-umask-stderr-output.txt";
        let status_path = "target/rubash-umask-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("umask -Z 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("rubash: umask: -Z: invalid option"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_umask_appends_stderr() {
        let error_path = "target/rubash-umask-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("umask -Z 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("rubash: umask: -Z: invalid option"));
        let _ = fs::remove_file(error_path);
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
    fn test_times_ignores_non_option_arguments() {
        let output_path = "target/rubash-times-extra-args-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("times ignored > {output_path}; echo $? >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n0\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_times_redirects_stderr() {
        let error_path = "target/rubash-times-stderr-output.txt";
        let status_path = "target/rubash-times-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("times -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("rubash: times: -x: invalid option"));
        assert!(error.contains("times: usage: times"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_times_appends_stderr() {
        let error_path = "target/rubash-times-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("times -x 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 2);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("rubash: times: -x: invalid option"));
        assert!(error.contains("times: usage: times"));
        let _ = fs::remove_file(error_path);
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
    fn test_help_redirects_stderr() {
        let error_path = "target/rubash-help-stderr-output.txt";
        let status_path = "target/rubash-help-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("help -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("help: -x: invalid option"));
        assert!(error.contains("help: usage: help [-dms] [pattern ...]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_help_appends_stderr() {
        let error_path = "target/rubash-help-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("help -x 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 2);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("help: -x: invalid option"));
        assert!(error.contains("help: usage: help [-dms] [pattern ...]"));
        let _ = fs::remove_file(error_path);
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
    fn test_builtin_dirs_redirects_output() {
        let output_path = "target/rubash-builtin-dirs-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("PWD=/tmp/rubash-builtin-dirs builtin dirs > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "/tmp/rubash-builtin-dirs\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_dirstack_expands_as_dynamic_array() {
        let output_path = "target/rubash-dirstack-array-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "PWD=/tmp/rubash-dirstack; \
             printf '%s:%s:%s\\n' \"${{!DIRSTACK[@]}}\" \"${{#DIRSTACK[@]}}\" \"${{DIRSTACK[0]}}\" > {output_path}; \
             DIRSTACK[0]=/tmp/rubash-dirstack-updated; \
             printf '%s:%s:%s\\n' \"${{!DIRSTACK[@]}}\" \"${{#DIRSTACK[@]}}\" \"${{DIRSTACK[0]}}\" >> {output_path}; \
             declare -p DIRSTACK >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0:1:/tmp/rubash-dirstack\n0:1:/tmp/rubash-dirstack-updated\ndeclare -a DIRSTACK=([0]=\"/tmp/rubash-dirstack-updated\")\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_dirs_redirects_stderr() {
        let error_path = "target/rubash-dirs-stderr-output.txt";
        let status_path = "target/rubash-dirs-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("dirs bad 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("dirs: bad: invalid option"));
        assert!(error.contains("dirs: usage:"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_dirs_appends_stderr() {
        let error_path = "target/rubash-dirs-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("dirs bad 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("dirs: bad: invalid option"));
        assert!(error.contains("dirs: usage:"));
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_pushd_redirects_stderr() {
        let error_path = "target/rubash-pushd-stderr-output.txt";
        let status_path = "target/rubash-pushd-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("pushd +9 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("pushd: +9: directory stack index out of range"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_pushd_n_accepts_double_dash_before_directory() {
        let output_path = "target/rubash-pushd-n-double-dash-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("PWD=/; pushd -n -- /tmp > {output_path}; dirs -p >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "/ /tmp\n/\n/tmp\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_popd_appends_stderr() {
        let error_path = "target/rubash-popd-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("popd +9 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("popd: +9: directory stack index out of range"));
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_popd_n_removes_next_directory_without_cd() {
        let output_path = "target/rubash-popd-n-output.txt";
        let scratch_path = "target/rubash-popd-n-scratch.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(scratch_path);
        let input = format!(
            "PWD=/; pushd -n /tmp > {scratch_path}; popd -n -- > {output_path}; dirs -p >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "/\n/\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(scratch_path);
    }

    #[test]
    fn test_popd_n_empty_stack_returns_failure() {
        let error_path = "target/rubash-popd-n-empty-error.txt";
        let status_path = "target/rubash-popd-n-empty-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("PWD=/; popd -n 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("popd: directory stack empty"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
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
    fn test_kill_translates_exit_signal() {
        let output_path = "target/rubash-kill-exit-signal-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("kill -l 0 > {output_path}; kill -l EXIT >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "EXIT\n0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_kill_translates_int_and_term_signals() {
        let output_path = "target/rubash-kill-common-signals-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "kill -l 2 > {output_path}; kill -l 130 >> {output_path}; \
             kill -l TERM >> {output_path}; kill -l 15 >> {output_path}; \
             kill -l 143 >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "INT\nINT\n15\nTERM\nTERM\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_kill_translates_quit_and_kill_signals() {
        let output_path = "target/rubash-kill-quit-kill-signals-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "kill -l 3 > {output_path}; kill -l 131 >> {output_path}; \
             kill -l QUIT >> {output_path}; kill -l 9 >> {output_path}; \
             kill -l 137 >> {output_path}; kill -l KILL >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "QUIT\nQUIT\n3\nKILL\nKILL\n9\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_kill_translates_more_common_signals() {
        let output_path = "target/rubash-kill-more-common-signals-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "kill -l 6 > {output_path}; kill -l 134 >> {output_path}; \
             kill -l SIGABRT >> {output_path}; kill -l PIPE >> {output_path}; \
             kill -l 141 >> {output_path}; kill -l ALRM >> {output_path}; \
             kill -l 142 >> {output_path}; kill -l SIGSEGV >> {output_path}; \
             kill -l 139 >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "ABRT\nABRT\n6\n13\nPIPE\n14\nALRM\n11\nSEGV\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_kill_translates_realtime_signals() {
        let output_path = "target/rubash-kill-realtime-signals-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "kill -l 32 > {output_path}; kill -l 160 >> {output_path}; \
             kill -l SIGRTMIN >> {output_path}; kill -l RTMIN+1 >> {output_path}; \
             kill -l SIGRTMIN+1 >> {output_path}; kill -l 49 >> {output_path}; \
             kill -l RTMAX-1 >> {output_path}; kill -l SIGRTMAX >> {output_path}; \
             kill -l 64 >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "RTMIN\nRTMIN\n32\n33\n33\nRTMAX-15\n63\n64\nRTMAX\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_kill_lists_common_signals() {
        let output_path = "target/rubash-kill-list-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("kill -l > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("1) SIGHUP"));
        assert!(output.contains("3) SIGQUIT"));
        assert!(output.contains("9) SIGKILL"));
        assert!(output.contains("15) SIGTERM"));
        assert!(output.contains("31) SIGUSR2"));
        assert!(output.contains("32) SIGRTMIN"));
        assert!(output.contains("64) SIGRTMAX"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_kill_redirects_stderr() {
        let error_path = "target/rubash-kill-stderr-output.txt";
        let status_path = "target/rubash-kill-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("kill -l NO_SUCH_SIGNAL 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("kill: NO_SUCH_SIGNAL: invalid signal specification"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_kill_appends_stderr() {
        let error_path = "target/rubash-kill-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("kill -l NO_SUCH_SIGNAL 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("kill: NO_SUCH_SIGNAL: invalid signal specification"));
        let _ = fs::remove_file(error_path);
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
    fn test_ulimit_all_lists_resource_limits() {
        let output_path = "target/rubash-ulimit-all-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("ulimit -n 2048; ulimit -a > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("core file size"));
        assert!(output.contains("(blocks, -f) unlimited"));
        assert!(output.contains("open files"));
        assert!(output.contains("(-n) 2048"));
        assert!(output.contains("virtual memory"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_ulimit_redirects_stderr() {
        let error_path = "target/rubash-ulimit-stderr-output.txt";
        let status_path = "target/rubash-ulimit-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("ulimit -g 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("ulimit: -g: invalid option"));
        assert!(error.contains("ulimit: usage:"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_ulimit_appends_stderr() {
        let error_path = "target/rubash-ulimit-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("ulimit -g 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("ulimit: -g: invalid option"));
        assert!(error.contains("ulimit: usage:"));
        let _ = fs::remove_file(error_path);
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
    fn test_builtin_alias_updates_alias_table() {
        let output_path = "target/rubash-builtin-alias-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin alias ll='ls -l'; builtin alias -p > {output_path}");
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
    fn test_builtin_unalias_updates_alias_table() {
        let output_path = "target/rubash-builtin-unalias-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "builtin alias gone='echo bad'; builtin unalias gone; alias -p > {output_path}"
        );
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
    fn test_alias_rejects_invalid_option() {
        let error_path = "target/rubash-alias-invalid-option-error.txt";
        let status_path = "target/rubash-alias-invalid-option-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("alias -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("alias: -x: invalid option"));
        assert!(error.contains("alias: usage: alias [-p] [name[=value] ... ]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_alias_accepts_double_dash_before_operand() {
        let output_path = "target/rubash-alias-double-dash-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("alias -- a='echo ok'; alias a > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "alias a='echo ok'\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_unalias_a_clears_all_and_ignores_operands() {
        let output_path = "target/rubash-unalias-a-output.txt";
        let status_path = "target/rubash-unalias-a-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!(
            "alias a=1 b=2; unalias -a a; echo $? > {status_path}; alias -p > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        assert_eq!(fs::read_to_string(output_path).unwrap(), "");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_unalias_rejects_invalid_option() {
        let error_path = "target/rubash-unalias-invalid-option-error.txt";
        let status_path = "target/rubash-unalias-invalid-option-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("unalias -x a 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("unalias: -x: invalid option"));
        assert!(error.contains("unalias: usage: unalias [-a] name [name ...]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_unalias_accepts_double_dash_before_dash_name() {
        let status_path = "target/rubash-unalias-double-dash-status.txt";
        let _ = fs::remove_file(status_path);
        let input = format!("alias -- -a=1; unalias -- -a; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_alias_redirects_stderr() {
        let error_path = "target/rubash-alias-stderr-output.txt";
        let status_path = "target/rubash-alias-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("alias no_such_alias 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("alias: no_such_alias: not found"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_alias_appends_stderr() {
        let error_path = "target/rubash-alias-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("alias no_such_alias 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("alias: no_such_alias: not found"));
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_unalias_redirects_stderr() {
        let error_path = "target/rubash-unalias-stderr-output.txt";
        let status_path = "target/rubash-unalias-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("unalias no_such_alias 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("unalias: no_such_alias: not found"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_unalias_appends_stderr() {
        let error_path = "target/rubash-unalias-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("unalias no_such_alias 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("unalias: no_such_alias: not found"));
        let _ = fs::remove_file(error_path);
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
    fn test_set_redirects_stderr() {
        let error_path = "target/rubash-set-stderr-output.txt";
        let status_path = "target/rubash-set-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("set -Z 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("rubash: set: -Z: invalid option"));
        assert!(error.contains("set: usage:"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_set_appends_stderr() {
        let error_path = "target/rubash-set-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("set -Z 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("rubash: set: -Z: invalid option"));
        assert!(error.contains("set: usage:"));
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_unset_redirects_stderr() {
        let error_path = "target/rubash-unset-stderr-output.txt";
        let status_path = "target/rubash-unset-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("unset 1BAD 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("rubash: unset: `1BAD`: not a valid identifier"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_unset_appends_stderr() {
        let error_path = "target/rubash-unset-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("unset 1BAD 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("rubash: unset: `1BAD`: not a valid identifier"));
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_builtin_unset_removes_variable() {
        let output_path = "target/rubash-builtin-unset-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("RUBASH_BUILTIN_UNSET=value; builtin unset RUBASH_BUILTIN_UNSET; echo ${{RUBASH_BUILTIN_UNSET:-missing}} > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "missing\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_unset_invalid_option_redirects_stderr() {
        let error_path = "target/rubash-unset-invalid-option-stderr-output.txt";
        let status_path = "target/rubash-unset-invalid-option-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("unset -Z 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("unset: -Z: invalid option"));
        assert!(error.contains("unset: usage:"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_unset_conflicting_options_append_stderr() {
        let error_path = "target/rubash-unset-conflicting-options-stderr-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("unset -fv value 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("unset: cannot simultaneously unset a function and a variable"));
        let _ = fs::remove_file(error_path);
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
    fn test_enable_p_lists_enabled_builtins() {
        let output_path = "target/rubash-enable-p-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("enable -p > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("enable echo\n"));
        assert!(output.contains("enable test\n"));
        assert!(output.contains("enable source\n"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_enable_a_marks_disabled_builtins() {
        let output_path = "target/rubash-enable-a-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("enable -n echo; enable -a > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("enable -n echo\n"));
        assert!(output.contains("enable test\n"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_enable_updates_disabled_builtin_state() {
        let bin_dir = target_test_path("rubash-builtin-enable-bin");
        #[cfg(windows)]
        let script_path = bin_dir.join("test.cmd");
        #[cfg(not(windows))]
        let script_path = bin_dir.join("test");
        let output_path = target_test_path("rubash-builtin-enable-output.txt");
        let shell_bin_dir = shell_test_path(&bin_dir);
        let shell_output_path = shell_test_path(&output_path);
        let _ = fs::remove_dir_all(&bin_dir);
        let _ = fs::remove_file(&output_path);
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(&script_path, "echo external-test\n").unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&script_path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script_path, permissions).unwrap();
        }
        let input = format!("builtin enable -n test; type -t test > {shell_output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("PATH", &shell_bin_dir);

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "file\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir_all(bin_dir);
    }

    #[test]
    fn test_disabled_test_builtin_uses_external_command() {
        let bin_dir = "target/rubash-disabled-test-bin";
        let script_path = format!("{bin_dir}/test");
        let output_path = "target/rubash-disabled-test-output.txt";
        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&script_path, "echo external-test\n").unwrap();
        let input = format!(
            "enable -n test; test > {output_path}; enable test; test 1 -eq 1; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "external-test\n0\n"
        );
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_command_uses_external_test_when_builtin_is_disabled() {
        let bin_dir = "target/rubash-disabled-command-test-bin";
        let script_path = format!("{bin_dir}/test");
        let output_path = "target/rubash-disabled-command-test-output.txt";
        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&script_path, "echo external-command-test\n").unwrap();
        let input = format!("enable -n test; command test > {output_path}; enable test");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "external-command-test\n"
        );
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_disabled_read_builtins_use_external_commands() {
        let bin_dir = "target/rubash-disabled-read-bin";
        let read_path = format!("{bin_dir}/read");
        let mapfile_path = format!("{bin_dir}/mapfile");
        let readarray_path = format!("{bin_dir}/readarray");
        let read_output_path = "target/rubash-disabled-read-output.txt";
        let command_read_output_path = "target/rubash-disabled-command-read-output.txt";
        let mapfile_output_path = "target/rubash-disabled-mapfile-output.txt";
        let readarray_output_path = "target/rubash-disabled-readarray-output.txt";
        let _ = fs::remove_dir_all(bin_dir);
        for path in [
            read_output_path,
            command_read_output_path,
            mapfile_output_path,
            readarray_output_path,
        ] {
            let _ = fs::remove_file(path);
        }
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&read_path, "echo external-read\n").unwrap();
        fs::write(&mapfile_path, "echo external-mapfile\n").unwrap();
        fs::write(&readarray_path, "echo external-readarray\n").unwrap();
        let input = format!(
            "enable -n read mapfile readarray; \
             read value <<< hi > {read_output_path}; \
             command read value <<< hi > {command_read_output_path}; \
             mapfile arr <<< hi > {mapfile_output_path}; \
             readarray arr <<< hi > {readarray_output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(read_output_path).unwrap(),
            "external-read\n"
        );
        assert_eq!(
            fs::read_to_string(command_read_output_path).unwrap(),
            "external-read\n"
        );
        assert_eq!(
            fs::read_to_string(mapfile_output_path).unwrap(),
            "external-mapfile\n"
        );
        assert_eq!(
            fs::read_to_string(readarray_output_path).unwrap(),
            "external-readarray\n"
        );
        let _ = fs::remove_dir_all(bin_dir);
        for path in [
            read_output_path,
            command_read_output_path,
            mapfile_output_path,
            readarray_output_path,
        ] {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn test_disabled_echo_builtin_uses_external_command() {
        let bin_dir = "target/rubash-disabled-echo-bin";
        let script_path = format!("{bin_dir}/echo");
        let output_path = "target/rubash-disabled-echo-output.txt";
        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&script_path, "printf 'external-echo %s\\n' \"$*\"\n").unwrap();
        let input = format!(
            "enable -n echo; echo hello > {output_path}; enable echo; echo builtin >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "external-echo hello\nbuiltin\n"
        );
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_command_uses_external_echo_when_builtin_is_disabled() {
        let bin_dir = "target/rubash-disabled-command-echo-bin";
        let script_path = format!("{bin_dir}/echo");
        let output_path = "target/rubash-disabled-command-echo-output.txt";
        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(
            &script_path,
            "printf 'external-command-echo %s\\n' \"$*\"\n",
        )
        .unwrap();
        let input = format!("enable -n echo; command echo hello > {output_path}; enable echo");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "external-command-echo hello\n"
        );
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_disabled_printf_builtin_uses_external_command() {
        let bin_dir = "target/rubash-disabled-printf-bin";
        let script_path = format!("{bin_dir}/printf");
        let output_path = "target/rubash-disabled-printf-output.txt";
        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&script_path, "echo external-printf \"$@\"\n").unwrap();
        let input = format!(
            "enable -n printf; printf hello > {output_path}; enable printf; printf '%s\\n' builtin >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "external-printf hello\nbuiltin\n"
        );
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_command_uses_external_printf_when_builtin_is_disabled() {
        let bin_dir = "target/rubash-disabled-command-printf-bin";
        let script_path = format!("{bin_dir}/printf");
        let output_path = "target/rubash-disabled-command-printf-output.txt";
        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&script_path, "echo external-command-printf \"$@\"\n").unwrap();
        let input =
            format!("enable -n printf; command printf hello > {output_path}; enable printf");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "external-command-printf hello\n"
        );
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_disabled_pwd_builtin_uses_external_command() {
        let bin_dir = "target/rubash-disabled-pwd-bin";
        let script_path = format!("{bin_dir}/pwd");
        let output_path = "target/rubash-disabled-pwd-output.txt";
        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&script_path, "echo external-pwd\n").unwrap();
        let input = format!("enable -n pwd; pwd > {output_path}; enable pwd");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "external-pwd\n");
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_command_uses_external_pwd_when_builtin_is_disabled() {
        let bin_dir = "target/rubash-disabled-command-pwd-bin";
        let script_path = format!("{bin_dir}/pwd");
        let output_path = "target/rubash-disabled-command-pwd-output.txt";
        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&script_path, "echo external-command-pwd\n").unwrap();
        let input = format!("enable -n pwd; command pwd > {output_path}; enable pwd");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "external-command-pwd\n"
        );
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_disabled_status_and_state_builtins_use_external_commands() {
        let bin_dir = "target/rubash-disabled-status-builtins-bin";
        let _ = fs::remove_dir_all(bin_dir);
        for path in [
            "target/rubash-disabled-true-output.txt",
            "target/rubash-disabled-false-output.txt",
            "target/rubash-disabled-hash-output.txt",
            "target/rubash-disabled-umask-output.txt",
            "target/rubash-disabled-command-true-output.txt",
            "target/rubash-disabled-command-false-output.txt",
            "target/rubash-disabled-command-hash-output.txt",
            "target/rubash-disabled-command-umask-output.txt",
        ] {
            let _ = fs::remove_file(path);
        }
        fs::create_dir_all(bin_dir).unwrap();
        for name in ["true", "false", "hash", "umask"] {
            fs::write(
                format!("{bin_dir}/{name}"),
                format!("echo external-{name}\n"),
            )
            .unwrap();
        }
        let input = format!(
            "enable -n true false hash umask; \
             true > target/rubash-disabled-true-output.txt; \
             false > target/rubash-disabled-false-output.txt; \
             hash > target/rubash-disabled-hash-output.txt; \
             umask > target/rubash-disabled-umask-output.txt; \
             command true > target/rubash-disabled-command-true-output.txt; \
             command false > target/rubash-disabled-command-false-output.txt; \
             command hash > target/rubash-disabled-command-hash-output.txt; \
             command umask > target/rubash-disabled-command-umask-output.txt; \
             enable true false hash umask"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        for (path, expected) in [
            ("target/rubash-disabled-true-output.txt", "external-true\n"),
            (
                "target/rubash-disabled-false-output.txt",
                "external-false\n",
            ),
            ("target/rubash-disabled-hash-output.txt", "external-hash\n"),
            (
                "target/rubash-disabled-umask-output.txt",
                "external-umask\n",
            ),
            (
                "target/rubash-disabled-command-true-output.txt",
                "external-true\n",
            ),
            (
                "target/rubash-disabled-command-false-output.txt",
                "external-false\n",
            ),
            (
                "target/rubash-disabled-command-hash-output.txt",
                "external-hash\n",
            ),
            (
                "target/rubash-disabled-command-umask-output.txt",
                "external-umask\n",
            ),
        ] {
            assert_eq!(fs::read_to_string(path).unwrap(), expected);
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir_all(bin_dir);
    }

    #[test]
    fn test_builtin_command_respects_disabled_builtin_state() {
        let output_path = "target/rubash-disabled-builtin-direct-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "enable -n true hash; \
             builtin true > {output_path}; \
             echo true:$? >> {output_path}; \
             builtin hash >> {output_path}; \
             echo hash:$? >> {output_path}; \
             enable true hash"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "true:1\nhash:1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_disabled_builtin_dispatch_uses_external_commands() {
        let bin_dir = "target/rubash-disabled-builtin-dispatch-bin";
        let outputs = [
            "target/rubash-disabled-cd-output.txt",
            "target/rubash-disabled-alias-output.txt",
            "target/rubash-disabled-type-output.txt",
            "target/rubash-disabled-command-cd-output.txt",
            "target/rubash-disabled-command-alias-output.txt",
            "target/rubash-disabled-command-type-output.txt",
            "target/rubash-disabled-command-builtin-output.txt",
        ];
        let _ = fs::remove_dir_all(bin_dir);
        for path in outputs {
            let _ = fs::remove_file(path);
        }
        fs::create_dir_all(bin_dir).unwrap();
        for name in ["cd", "alias", "type", "command"] {
            fs::write(
                format!("{bin_dir}/{name}"),
                format!("echo external-{name}\n"),
            )
            .unwrap();
        }
        let input = "\
            enable -n cd alias type; \
            cd > target/rubash-disabled-cd-output.txt; \
            alias > target/rubash-disabled-alias-output.txt; \
            type echo > target/rubash-disabled-type-output.txt; \
            command cd > target/rubash-disabled-command-cd-output.txt; \
            command alias > target/rubash-disabled-command-alias-output.txt; \
            command type echo > target/rubash-disabled-command-type-output.txt; \
            enable -n command; \
            command echo > target/rubash-disabled-command-builtin-output.txt";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        for (path, expected) in [
            ("target/rubash-disabled-cd-output.txt", "external-cd\n"),
            (
                "target/rubash-disabled-alias-output.txt",
                "external-alias\n",
            ),
            ("target/rubash-disabled-type-output.txt", "external-type\n"),
            (
                "target/rubash-disabled-command-cd-output.txt",
                "external-cd\n",
            ),
            (
                "target/rubash-disabled-command-alias-output.txt",
                "external-alias\n",
            ),
            (
                "target/rubash-disabled-command-type-output.txt",
                "external-type\n",
            ),
            (
                "target/rubash-disabled-command-builtin-output.txt",
                "external-command\n",
            ),
        ] {
            assert_eq!(fs::read_to_string(path).unwrap(), expected);
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir_all(bin_dir);
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
    fn test_enable_redirects_stderr() {
        let error_path = "target/rubash-enable-stderr-output.txt";
        let status_path = "target/rubash-enable-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("enable no_such_builtin 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("enable: no_such_builtin: not a shell builtin"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_enable_appends_stderr() {
        let error_path = "target/rubash-enable-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("enable no_such_builtin 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("enable: no_such_builtin: not a shell builtin"));
        let _ = fs::remove_file(error_path);
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
    fn test_builtin_type_invokes_type_builtin() {
        let output_path = "target/rubash-builtin-type-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "function type {{ echo function-type; }}; builtin type -t echo > {output_path}"
        );
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
    fn test_builtin_test_invokes_test_builtin() {
        let output_path = "target/rubash-builtin-test-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin test 3 -eq 3; echo $? > {output_path}");
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
    fn test_builtin_bracket_invokes_test_builtin() {
        let output_path = "target/rubash-builtin-bracket-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin [ 3 -eq 4 ]; echo $? > {output_path}");
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
    fn test_builtin_test_redirect_truncates_output_file() {
        let output_path = "target/rubash-builtin-test-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "before\n").unwrap();
        let input = format!("builtin test 3 -eq 3 > {output_path}; echo $? >> {output_path}");
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
    fn test_builtin_let_invokes_let_builtin() {
        let output_path = "target/rubash-builtin-let-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin let n=1+2 n; echo $? $n > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0 3\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_read_uses_here_string() {
        let output_path = "target/rubash-builtin-read-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("builtin read left right <<< 'alpha beta'; echo $left/$right > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha/beta\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_mapfile_uses_here_string() {
        let output_path = "target/rubash-builtin-mapfile-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "builtin mapfile -t arr <<< $'alpha\\nbeta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
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
    fn test_builtin_umask_redirects_output() {
        let output_path = "target/rubash-builtin-umask-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin umask 077; builtin umask > {output_path}");
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
    fn test_builtin_trap_redirects_output() {
        let output_path = "target/rubash-builtin-trap-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin trap 'echo bye' EXIT; builtin trap -p EXIT > {output_path}");
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
    fn test_builtin_source_updates_current_shell() {
        let script_path = "target/rubash-builtin-source-script.sh";
        let output_path = "target/rubash-builtin-source-output.txt";
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
        fs::write(script_path, "RUBASH_BUILTIN_SOURCE=ok\n").unwrap();
        let input =
            format!("builtin source {script_path}; echo $RUBASH_BUILTIN_SOURCE > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_dot_updates_current_shell() {
        let script_path = "target/rubash-builtin-dot-script.sh";
        let output_path = "target/rubash-builtin-dot-output.txt";
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
        fs::write(script_path, "RUBASH_BUILTIN_DOT=ok\n").unwrap();
        let input = format!("builtin . {script_path}; echo $RUBASH_BUILTIN_DOT > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_source_searches_path_before_current_directory() {
        let local_script = "rubash-sourcepath-temp.sh";
        let bin_dir = "target/rubash-sourcepath-bin";
        let path_script = format!("{bin_dir}/{local_script}");
        let output_path = "target/rubash-sourcepath-output.txt";
        let _ = fs::remove_file(local_script);
        let _ = fs::remove_file(&path_script);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(local_script, "RUBASH_SOURCEPATH=local\n").unwrap();
        fs::write(&path_script, "RUBASH_SOURCEPATH=path\n").unwrap();
        let input = format!(
            "PATH={bin_dir}; source {local_script}; echo $RUBASH_SOURCEPATH > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "path\n");
        let _ = fs::remove_file(local_script);
        let _ = fs::remove_file(path_script);
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_sourcepath_disabled_uses_current_directory() {
        let local_script = "rubash-sourcepath-disabled-temp.sh";
        let bin_dir = "target/rubash-sourcepath-disabled-bin";
        let path_script = format!("{bin_dir}/{local_script}");
        let output_path = "target/rubash-sourcepath-disabled-output.txt";
        let _ = fs::remove_file(local_script);
        let _ = fs::remove_file(&path_script);
        let _ = fs::remove_file(output_path);
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(local_script, "RUBASH_SOURCEPATH_DISABLED=local\n").unwrap();
        fs::write(&path_script, "RUBASH_SOURCEPATH_DISABLED=path\n").unwrap();
        let input = format!(
            "PATH={bin_dir}; shopt -u sourcepath; source {local_script}; shopt -s sourcepath; echo $RUBASH_SOURCEPATH_DISABLED > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "local\n");
        let _ = fs::remove_file(local_script);
        let _ = fs::remove_file(path_script);
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_dir(bin_dir);
    }

    #[test]
    fn test_source_p_empty_path_uses_current_directory() {
        let script_path = "rubash-source-p-empty-temp.sh";
        let output_path = "target/rubash-source-p-empty-output.txt";
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
        fs::write(script_path, "RUBASH_SOURCE_P_EMPTY=ok\n").unwrap();
        let input = format!(
            "PATH=/no/such/path; source -p '' {script_path}; echo $RUBASH_SOURCE_P_EMPTY > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
        let _ = fs::remove_file(script_path);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_source_option_errors_return_usage_status() {
        let output_path = "target/rubash-source-option-errors-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "source; echo nofile:$? > {output_path}; \
             source -p; echo missingpath:$? >> {output_path}; \
             . -i; echo invalid:$? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "nofile:2\nmissingpath:2\ninvalid:2\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_return_returns_from_function() {
        let output_path = "target/rubash-builtin-return-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "f() {{ builtin return 6; echo bad > {output_path}; }}; f; echo $? > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "6\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_caller_top_level_returns_failure() {
        let output_path = "target/rubash-caller-top-output.txt";
        let status_path = "target/rubash-caller-top-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!("caller > {output_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "");
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_caller_reports_current_function_call_site() {
        let output_path = "target/rubash-caller-current-output.txt";
        let status_path = "target/rubash-caller-current-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!("f() {{ caller > {output_path}; echo $? > {status_path}; }}; f");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1 NULL\n");
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_caller_zero_reports_parent_function_frame() {
        let output_path = "target/rubash-caller-zero-output.txt";
        let status_path = "target/rubash-caller-zero-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!(
            "f() {{ caller 0 > {output_path}; echo $? > {status_path}; }}; g() {{ f; }}; g"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "1 g environment\n"
        );
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_caller_invalid_argument_returns_usage() {
        let error_path = "target/rubash-caller-invalid-error.txt";
        let status_path = "target/rubash-caller-invalid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("f() {{ caller nope 2> {error_path}; echo $? > {status_path}; }}; f");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("caller: nope: invalid number"));
        assert!(error.contains("caller: usage: caller [expr]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_jobs_without_jobs_returns_success() {
        let output_path = "target/rubash-jobs-empty-output.txt";
        let status_path = "target/rubash-jobs-empty-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!("jobs > {output_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "");
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_jobs_invalid_option_returns_usage() {
        let error_path = "target/rubash-jobs-invalid-error.txt";
        let status_path = "target/rubash-jobs-invalid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("jobs -z 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("jobs: -z: invalid option"));
        assert!(error.contains("jobs: usage: jobs [-lnprs]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_jobs_x_executes_command() {
        let output_path = "target/rubash-jobs-x-output.txt";
        let status_path = "target/rubash-jobs-x-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!("jobs -x echo hi there > {output_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "hi there\n");
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_jobs_x_uses_command_status() {
        let status_path = "target/rubash-jobs-x-status-output.txt";
        let _ = fs::remove_file(status_path);
        let input = format!("jobs -x false; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_disown_without_jobs_reports_current_job_failure() {
        let error_path = "target/rubash-disown-empty-error.txt";
        let status_path = "target/rubash-disown-empty-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("disown 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        assert!(fs::read_to_string(error_path)
            .unwrap()
            .contains("disown: current: no such job"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_disown_all_or_running_without_jobs_succeeds() {
        let status_path = "target/rubash-disown-all-status.txt";
        let _ = fs::remove_file(status_path);
        let input =
            format!("disown -a; echo $? > {status_path}; disown -r; echo $? >> {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n0\n");
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_disown_invalid_option_returns_usage() {
        let error_path = "target/rubash-disown-invalid-error.txt";
        let status_path = "target/rubash-disown-invalid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("disown -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("disown: -x: invalid option"));
        assert!(error.contains("disown: usage: disown [-h] [-ar]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_builtin_and_command_disown_use_shell_builtin() {
        let status_path = "target/rubash-disown-builtin-status.txt";
        let _ = fs::remove_file(status_path);
        let input = format!(
            "builtin disown -a; echo builtin:$? > {status_path}; \
             command disown -a; echo command:$? >> {status_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(status_path).unwrap(),
            "builtin:0\ncommand:0\n"
        );
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_logout_non_login_shell_reports_error_and_continues() {
        let error_path = "target/rubash-logout-error.txt";
        let output_path = "target/rubash-logout-output.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(output_path);
        let input = format!(
            "logout 2> {error_path}; echo status:$? > {output_path}; echo after >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "status:1\nafter\n"
        );
        assert!(fs::read_to_string(error_path)
            .unwrap()
            .contains("logout: not login shell: use `exit'"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_and_command_logout_use_shell_builtin() {
        let error_path = "target/rubash-logout-builtin-error.txt";
        let output_path = "target/rubash-logout-builtin-output.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(output_path);
        let input = format!(
            "builtin logout 2> {error_path}; echo builtin:$? > {output_path}; \
             command logout 2>> {error_path}; echo command:$? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "builtin:1\ncommand:1\n"
        );
        let error = fs::read_to_string(error_path).unwrap();
        assert_eq!(
            error.matches("logout: not login shell: use `exit'").count(),
            2
        );
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_wait_without_operands_returns_success() {
        let status_path = "target/rubash-wait-empty-status.txt";
        let _ = fs::remove_file(status_path);
        let input = format!("wait; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_wait_for_unknown_pid_returns_notfound() {
        let error_path = "target/rubash-wait-pid-error.txt";
        let status_path = "target/rubash-wait-pid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("wait 999999 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "127\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("wait: pid 999999 is not a child of this shell"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_wait_invalid_operand_returns_failure() {
        let error_path = "target/rubash-wait-invalid-error.txt";
        let status_path = "target/rubash-wait-invalid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("wait abc 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("wait: `abc': not a pid or valid job spec"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_wait_invalid_option_returns_usage() {
        let error_path = "target/rubash-wait-invalid-option-error.txt";
        let status_path = "target/rubash-wait-invalid-option-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("wait -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("wait: -x: invalid option"));
        assert!(error.contains("wait: usage: wait [-fn] [-p var] [id ...]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_fg_without_job_control_returns_failure() {
        let error_path = "target/rubash-fg-error.txt";
        let status_path = "target/rubash-fg-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("fg 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("fg: no job control"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_bg_without_job_control_returns_failure() {
        let error_path = "target/rubash-bg-error.txt";
        let status_path = "target/rubash-bg-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("bg 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("bg: no job control"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_suspend_without_job_control_returns_failure() {
        let error_path = "target/rubash-suspend-error.txt";
        let status_path = "target/rubash-suspend-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("suspend 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("suspend: cannot suspend: no job control"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_suspend_invalid_option_returns_usage() {
        let error_path = "target/rubash-suspend-invalid-error.txt";
        let status_path = "target/rubash-suspend-invalid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("suspend -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("suspend: -x: invalid option"));
        assert!(error.contains("suspend: usage: suspend [-f]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_history_without_entries_returns_success() {
        let output_path = "target/rubash-history-empty-output.txt";
        let status_path = "target/rubash-history-empty-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!("history > {output_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "");
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_history_clear_returns_success() {
        let status_path = "target/rubash-history-clear-status.txt";
        let _ = fs::remove_file(status_path);
        let input = format!("history -c; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_history_invalid_option_returns_usage() {
        let error_path = "target/rubash-history-invalid-error.txt";
        let status_path = "target/rubash-history-invalid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("history -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("history: -x: invalid option"));
        assert!(error.contains("history: usage: history [-c] [-d offset]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_bind_warns_without_line_editing_and_succeeds() {
        let error_path = "target/rubash-bind-error.txt";
        let status_path = "target/rubash-bind-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("bind 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("bind: warning: line editing not enabled"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_bind_invalid_option_returns_usage() {
        let error_path = "target/rubash-bind-invalid-error.txt";
        let status_path = "target/rubash-bind-invalid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("bind -Z 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("bind: warning: line editing not enabled"));
        assert!(error.contains("bind: -Z: invalid option"));
        assert!(error.contains("bind: usage: bind [-lpsvPSVX]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_bind_option_requires_argument() {
        let error_path = "target/rubash-bind-missing-arg-error.txt";
        let status_path = "target/rubash-bind-missing-arg-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("bind -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("bind: -x: option requires an argument"));
        assert!(error.contains("bind: usage: bind [-lpsvPSVX]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_fc_without_history_returns_success() {
        let output_path = "target/rubash-fc-empty-output.txt";
        let status_path = "target/rubash-fc-empty-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!("fc > {output_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "");
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_fc_list_without_history_returns_success() {
        let output_path = "target/rubash-fc-list-output.txt";
        let status_path = "target/rubash-fc-list-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!("fc -l > {output_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "");
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_fc_invalid_option_returns_usage() {
        let error_path = "target/rubash-fc-invalid-error.txt";
        let status_path = "target/rubash-fc-invalid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("fc -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("fc: -x: invalid option"));
        assert!(error.contains("fc: usage: fc [-e ename] [-lnr]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_fc_edit_option_requires_argument() {
        let error_path = "target/rubash-fc-missing-edit-error.txt";
        let status_path = "target/rubash-fc-missing-edit-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("fc -e 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("fc: -e: option requires an argument"));
        assert!(error.contains("fc: usage: fc [-e ename] [-lnr]"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_builtin_break_breaks_loop() {
        let output_path = "target/rubash-builtin-break-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "for value in a b; do builtin break; echo bad; done; echo done > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "done\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_continue_continues_loop() {
        let output_path = "target/rubash-builtin-continue-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "for value in a b; do builtin continue; echo bad; done; echo done > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "done\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_kill_redirects_output() {
        let output_path = "target/rubash-builtin-kill-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin kill -l HUP > {output_path}");
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
    fn test_exec_redirects_output() {
        let output_path = "target/rubash-exec-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("exec -a custom sh -c 'echo $0' > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "custom\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_exec_combined_options_set_login_argv0() {
        let output_path = "target/rubash-exec-combined-options-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("exec -la custom sh -c 'echo $0' > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "-custom\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_exec_a_accepts_attached_argument() {
        let output_path = "target/rubash-exec-a-attached-argument-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("exec -acustom sh -c 'echo $0' > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "custom\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_exec_double_dash_stops_option_parsing() {
        let output_path = "target/rubash-exec-double-dash-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("exec -- sh -c 'echo $0' > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "sh\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_exec_child_environment_contains_only_exported_variables() {
        let output_path = target_test_path("rubash-exec-child-env-output.txt");
        let _ = fs::remove_file(&output_path);
        let shell_output_path = shell_test_path(&output_path);
        let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
        let input = format!(
            "RUBASH_EXEC_LOCAL=local; export RUBASH_EXEC_EXPORTED=exported; \
             exec {rubash} -c 'printf \"%s/%s\\n\" \"${{RUBASH_EXEC_LOCAL-unset}}\" \"$RUBASH_EXEC_EXPORTED\"' > {shell_output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(&output_path).unwrap(),
            "unset/exported\n"
        );
        std::env::remove_var("RUBASH_EXEC_LOCAL");
        std::env::remove_var("RUBASH_EXEC_EXPORTED");
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_exec_printenv_ignores_unexported_variable() {
        let output_path = target_test_path("rubash-exec-printenv-local-output.txt");
        let _ = fs::remove_file(&output_path);
        let shell_output_path = shell_test_path(&output_path);
        let input = format!("RUBASH_EXEC_PRINTENV_LOCAL=local; exec printenv RUBASH_EXEC_PRINTENV_LOCAL > {shell_output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
        assert_eq!(executor.last_exit_code(), 1);
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "");
        std::env::remove_var("RUBASH_EXEC_PRINTENV_LOCAL");
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_exec_printenv_reads_exported_variable() {
        let output_path = target_test_path("rubash-exec-printenv-exported-output.txt");
        let _ = fs::remove_file(&output_path);
        let shell_output_path = shell_test_path(&output_path);
        let input = format!("export RUBASH_EXEC_PRINTENV_EXPORTED=exported; exec printenv RUBASH_EXEC_PRINTENV_EXPORTED > {shell_output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "exported\n");
        std::env::remove_var("RUBASH_EXEC_PRINTENV_EXPORTED");
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_exec_runs_external_command() {
        let output_path = target_test_path("rubash-exec-runs-external-output.txt");
        #[cfg(windows)]
        let script_path = target_test_path("rubash-exec-runs-external.cmd");
        #[cfg(not(windows))]
        let script_path = target_test_path("rubash-exec-runs-external.sh");
        let shell_output_path = shell_test_path(&output_path);
        let shell_script_path = shell_test_path(&script_path);
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_file(&script_path);
        #[cfg(windows)]
        fs::write(&script_path, "@echo off\r\necho exec-ran\r\n").unwrap();
        #[cfg(not(windows))]
        fs::write(&script_path, "#!/bin/sh\nprintf '%s\\n' exec-ran\n").unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&script_path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script_path, permissions).unwrap();
        }
        let input = format!("exec {shell_script_path} > {shell_output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(&output_path)
                .unwrap()
                .replace("\r\n", "\n"),
            "exec-ran\n"
        );
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(script_path);
    }

    #[test]
    fn test_exec_stops_after_successful_command() {
        let output_path = target_test_path("rubash-exec-stops-output.txt");
        #[cfg(windows)]
        let script_path = target_test_path("rubash-exec-stops.cmd");
        #[cfg(not(windows))]
        let script_path = target_test_path("rubash-exec-stops.sh");
        let shell_output_path = shell_test_path(&output_path);
        let shell_script_path = shell_test_path(&script_path);
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_file(&script_path);
        #[cfg(windows)]
        fs::write(&script_path, "@echo off\r\necho exec-only\r\n").unwrap();
        #[cfg(not(windows))]
        fs::write(&script_path, "#!/bin/sh\nprintf '%s\\n' exec-only\n").unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&script_path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script_path, permissions).unwrap();
        }
        let input = format!(
            "exec {shell_script_path} > {shell_output_path}; echo after >> {shell_output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(&output_path)
                .unwrap()
                .replace("\r\n", "\n"),
            "exec-only\n"
        );
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(script_path);
    }

    #[test]
    fn test_exec_c_clears_external_command_environment() {
        let output_path = target_test_path("rubash-exec-clean-env-output.txt");
        let script_path = target_test_path("rubash-exec-clean-env.sh");
        #[cfg(windows)]
        let shell_path = std::path::PathBuf::from(
            std::env::var("CLAUDE_CODE_GIT_BASH_PATH")
                .unwrap_or_else(|_| r"D:\Git\bin\bash.exe".to_string()),
        );
        #[cfg(not(windows))]
        let shell_path = std::path::PathBuf::from("/bin/sh");
        let shell_output_path = shell_test_path(&output_path);
        let shell_script_path = shell_test_path(&script_path);
        let shell_command_path = shell_test_path(&shell_path);
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_file(&script_path);
        fs::write(
            &script_path,
            "if [ -n \"$FOO\" ]; then printf 'FOO=%s\\n' \"$FOO\"; else printf 'FOO=\\n'; fi\n",
        )
        .unwrap();

        let input =
            format!("exec -c {shell_command_path} {shell_script_path} > {shell_output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("FOO", "present");

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "FOO=\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(script_path);
    }

    #[test]
    fn test_exec_missing_command_returns_not_found() {
        let error_path = "target/rubash-exec-missing-command-error.txt";
        let _ = fs::remove_file(error_path);
        let input = format!("exec no_such_rubash_command 2> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(127))));
        assert_eq!(executor.last_exit_code(), 127);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("exec: no_such_rubash_command: not found"));
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_exec_invalid_option_redirects_stderr() {
        let output_path = "target/rubash-exec-invalid-option-output.txt";
        let error_path = "target/rubash-exec-invalid-option-error.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
        let input = format!("exec -Z 2> {error_path}; echo $? > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("exec: -Z: invalid option"));
        assert!(error.contains("exec: usage:"));
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_exec_a_requires_argument() {
        let output_path = "target/rubash-exec-a-requires-argument-output.txt";
        let error_path = "target/rubash-exec-a-requires-argument-error.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
        let input = format!("exec -a 2> {error_path}; echo $? > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("exec: -a: option requires an argument"));
        assert!(error.contains("exec: usage:"));
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_builtin_exec_redirects_output() {
        let output_path = "target/rubash-builtin-exec-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin exec -a custom sh -c 'echo $0' > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "custom\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_builtin_command_redirects_output() {
        let output_path = "target/rubash-builtin-command-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("builtin command echo hello > {output_path}");
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
    fn test_command_type_invokes_type_builtin() {
        let output_path = "target/rubash-command-type-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "function type {{ echo function-type; }}; command type -t echo > {output_path}"
        );
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
    fn test_command_test_invokes_test_builtin() {
        let output_path = "target/rubash-command-test-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("command test 3 -eq 4; echo $? > {output_path}");
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
    fn test_command_trap_redirects_output() {
        let output_path = "target/rubash-command-trap-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("trap 'echo bye' EXIT; command trap -p EXIT > {output_path}");
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
    fn test_command_kill_redirects_output() {
        let output_path = "target/rubash-command-kill-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("command kill -l HUP > {output_path}");
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
    fn test_command_declare_assigns_variable() {
        let output_path = "target/rubash-command-declare-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "command declare RUBASH_COMMAND_DECLARE=value; echo $RUBASH_COMMAND_DECLARE > {output_path}"
        );
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
    fn test_command_unset_removes_variable() {
        let output_path = "target/rubash-command-unset-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "RUBASH_COMMAND_UNSET=value; command unset RUBASH_COMMAND_UNSET; echo ${{RUBASH_COMMAND_UNSET:-missing}} > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "missing\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_command_shopt_updates_shell_option_state() {
        let output_path = "target/rubash-command-shopt-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("command shopt -s nullglob; shopt -q nullglob; echo $? > {output_path}");
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
    fn test_command_eval_redirects_output() {
        let output_path = "target/rubash-command-eval-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("command eval 'echo alpha; echo beta' > {output_path}");
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
    fn test_command_exec_redirects_output() {
        let output_path = "target/rubash-command-exec-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("command exec -a custom sh -c 'echo $0' > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(0))));
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "custom\n");
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
    fn test_builtin_eval_redirects_entire_output() {
        let output_path = "target/rubash-builtin-eval-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "old\n").unwrap();
        let input = format!("builtin eval 'echo alpha; echo beta' > {output_path}");
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
    fn test_complete_empty_state_redirects_no_output() {
        let output_path = "target/rubash-complete-output.txt";
        let status_path = "target/rubash-complete-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!("complete > {output_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "");
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_complete_invalid_option_reports_usage() {
        let error_path = "target/rubash-complete-error.txt";
        let status_path = "target/rubash-complete-error-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("complete -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("complete: -x: invalid option\n"));
        assert!(error.contains("complete: usage: complete "));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_compgen_empty_state_redirects_no_output() {
        let output_path = "target/rubash-compgen-output.txt";
        let status_path = "target/rubash-compgen-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!("compgen > {output_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "");
        assert_eq!(fs::read_to_string(status_path).unwrap(), "0\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_compgen_invalid_option_reports_usage() {
        let error_path = "target/rubash-compgen-error.txt";
        let status_path = "target/rubash-compgen-error-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("compgen -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("compgen: -x: invalid option\n"));
        assert!(error.contains("compgen: usage: compgen "));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_compopt_outside_completion_function_fails() {
        let error_path = "target/rubash-compopt-error.txt";
        let status_path = "target/rubash-compopt-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("compopt 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        assert!(fs::read_to_string(error_path)
            .unwrap()
            .contains("compopt: not currently executing completion function\n"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_builtin_compopt_invalid_option_reports_usage() {
        let error_path = "target/rubash-builtin-compopt-error.txt";
        let status_path = "target/rubash-builtin-compopt-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("builtin compopt -x 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("compopt: -x: invalid option\n"));
        assert!(error.contains("compopt: usage: compopt "));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
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
    fn test_command_p_uses_standard_path_for_external_command() {
        let output_path = "target/rubash-command-p-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("PATH=target/rubash-no-such-bin command -p sh -c 'echo ok' > {output_path}");
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
    fn test_command_p_v_uses_standard_path_for_external_command() {
        let output_path = "target/rubash-command-p-v-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("PATH=target/rubash-no-such-bin command -p -v sh > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.trim_end().ends_with("sh") || output.trim_end().ends_with("sh.exe"));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_command_v_without_p_uses_current_path_for_external_command() {
        let bin_dir = target_test_path("rubash-command-v-bin");
        let missing_bin_dir = target_test_path("rubash-no-such-bin");
        #[cfg(windows)]
        let script_path = bin_dir.join("sh.cmd");
        #[cfg(not(windows))]
        let script_path = bin_dir.join("sh");
        let status_path = target_test_path("rubash-command-v-without-p-status.txt");
        let output_path = target_test_path("rubash-command-v-without-p-output.txt");
        let restored_path = target_test_path("rubash-command-v-restored-output.txt");
        let restored_status_path = target_test_path("rubash-command-v-restored-status.txt");
        let shell_bin_dir = shell_test_path(&bin_dir);
        let shell_missing_bin_dir = shell_test_path(&missing_bin_dir);
        let shell_status_path = shell_test_path(&status_path);
        let shell_output_path = shell_test_path(&output_path);
        let shell_restored_path = shell_test_path(&restored_path);
        let shell_restored_status_path = shell_test_path(&restored_status_path);
        let _ = fs::remove_dir_all(&bin_dir);
        let _ = fs::remove_dir_all(&missing_bin_dir);
        let _ = fs::remove_file(&status_path);
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_file(&restored_path);
        let _ = fs::remove_file(&restored_status_path);
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(&script_path, "echo fake-sh\n").unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&script_path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script_path, permissions).unwrap();
        }
        let input = format!(
            "PATH={shell_missing_bin_dir} command -v sh > {shell_output_path}; \
             echo $? > {shell_status_path}; command -v sh > {shell_restored_path}; \
             echo $? > {shell_restored_status_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("PATH", &shell_bin_dir);

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "");
        assert_eq!(fs::read_to_string(&status_path).unwrap(), "1\n");
        assert!(!fs::read_to_string(&restored_path).unwrap().is_empty());
        assert_eq!(fs::read_to_string(&restored_status_path).unwrap(), "0\n");
        let _ = fs::remove_file(status_path);
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(restored_path);
        let _ = fs::remove_file(restored_status_path);
        let _ = fs::remove_dir_all(bin_dir);
    }

    #[test]
    fn test_describe_disabled_builtin_uses_path_command() {
        let bin_dir = "target/rubash-disabled-describe-bin";
        let script_path = format!("{bin_dir}/echo");
        let command_v_path = "target/rubash-disabled-command-v-output.txt";
        let command_v_verbose_path = "target/rubash-disabled-command-v-verbose-output.txt";
        let type_t_path = "target/rubash-disabled-type-t-output.txt";
        let type_verbose_path = "target/rubash-disabled-type-verbose-output.txt";
        let _ = fs::remove_dir_all(bin_dir);
        for path in [
            command_v_path,
            command_v_verbose_path,
            type_t_path,
            type_verbose_path,
        ] {
            let _ = fs::remove_file(path);
        }
        fs::create_dir_all(bin_dir).unwrap();
        fs::write(&script_path, "echo external-echo\n").unwrap();
        let input = format!(
            "enable -n echo; \
             command -v echo > {command_v_path}; \
             command -V echo > {command_v_verbose_path}; \
             type -t echo > {type_t_path}; \
             type echo > {type_verbose_path}; \
             enable echo"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let old_path = std::env::var("PATH").ok();
        executor.set_env("PATH", bin_dir);

        let result = executor.execute_ast(&ast);
        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let command_v = fs::read_to_string(command_v_path).unwrap();
        let command_v_verbose = fs::read_to_string(command_v_verbose_path).unwrap();
        let type_verbose = fs::read_to_string(type_verbose_path).unwrap();
        assert!(command_v.contains("rubash-disabled-describe-bin"));
        assert!(command_v.trim_end().ends_with("echo"));
        assert!(command_v_verbose.contains("echo is "));
        assert!(command_v_verbose.contains("rubash-disabled-describe-bin"));
        assert_eq!(fs::read_to_string(type_t_path).unwrap(), "file\n");
        assert!(type_verbose.contains("echo is "));
        assert!(type_verbose.contains("rubash-disabled-describe-bin"));
        let _ = fs::remove_dir_all(bin_dir);
        for path in [
            command_v_path,
            command_v_verbose_path,
            type_t_path,
            type_verbose_path,
        ] {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn test_nested_command_describe_uses_shell_state() {
        let builtin_function_path = "target/rubash-builtin-command-v-function-output.txt";
        let builtin_verbose_path = "target/rubash-builtin-command-v-verbose-output.txt";
        let nested_function_path = "target/rubash-nested-command-v-function-output.txt";
        let builtin_alias_path = "target/rubash-builtin-command-v-alias-output.txt";
        for path in [
            builtin_function_path,
            builtin_verbose_path,
            nested_function_path,
            builtin_alias_path,
        ] {
            let _ = fs::remove_file(path);
        }
        let input = format!(
            "function ff {{ echo f; }}; \
             shopt -s expand_aliases; \
             alias aa='echo alias value'; \
             builtin command -v ff > {builtin_function_path}; \
             builtin command -V ff > {builtin_verbose_path}; \
             command command -v ff > {nested_function_path}; \
             builtin command -v aa > {builtin_alias_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(builtin_function_path).unwrap(), "ff\n");
        let verbose = fs::read_to_string(builtin_verbose_path).unwrap();
        assert!(verbose.contains("ff is a function"));
        assert_eq!(fs::read_to_string(nested_function_path).unwrap(), "ff\n");
        assert_eq!(
            fs::read_to_string(builtin_alias_path).unwrap(),
            "alias aa='echo alias value'\n"
        );
        for path in [
            builtin_function_path,
            builtin_verbose_path,
            nested_function_path,
            builtin_alias_path,
        ] {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn test_command_without_p_uses_current_path_for_external_command() {
        let output_path = "target/rubash-command-without-p-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("PATH=target/rubash-no-such-bin command sh -c 'echo bad' > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 127);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "");
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
    fn test_trap_accepts_common_signal_names() {
        let output_path = "target/rubash-trap-common-signals-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "trap 'echo pipe' PIPE; trap 'echo alarm' 14; trap -p SIGPIPE ALRM > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "trap -- 'echo pipe' SIGPIPE\ntrap -- 'echo alarm' SIGALRM\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_trap_accepts_realtime_signal_names() {
        let output_path = "target/rubash-trap-realtime-signals-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("trap 'echo rt' RTMIN+1; trap -p SIGRTMIN+1 > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "trap -- 'echo rt' SIGRTMIN+1\n"
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
    fn test_trap_redirects_stderr() {
        let error_path = "target/rubash-trap-stderr-output.txt";
        let status_path = "target/rubash-trap-stderr-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input =
            format!("trap 'echo bad' NO_SUCH_SIGNAL 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("trap: NO_SUCH_SIGNAL: invalid signal specification"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_trap_appends_stderr() {
        let error_path = "target/rubash-trap-stderr-append-output.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "before\n").unwrap();
        let input = format!("trap 'echo bad' NO_SUCH_SIGNAL 2>> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.starts_with("before\n"));
        assert!(error.contains("trap: NO_SUCH_SIGNAL: invalid signal specification"));
        let _ = fs::remove_file(error_path);
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
    fn test_read_p_consumes_prompt_argument() {
        let output_path = "target/rubash-read-p-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("read -p prompt value <<< alpha; echo $value > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_read_i_consumes_initial_text_argument() {
        let output_path = "target/rubash-read-i-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("read -i seed value <<< alpha; echo $value > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_read_without_r_processes_backslash_escape() {
        let output_path = "target/rubash-read-backslash-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("read line <<< 'alpha\\beta'; echo $line > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "alphabeta\n");
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
    fn test_read_multiple_names_respects_backslash_escaped_ifs() {
        let output_path = "target/rubash-read-escaped-ifs-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("read first rest <<< 'alpha\\ beta gamma'; printf '<%s><%s>\\n' \"$first\" \"$rest\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "<alpha beta><gamma>\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_read_r_multiple_names_treats_backslash_as_literal() {
        let output_path = "target/rubash-read-r-multiple-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "read -r first rest <<< 'alpha\\ beta gamma'; printf '<%s><%s>\\n' \"$first\" \"$rest\" > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "<alpha\\><beta gamma>\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_read_combined_silent_raw_treats_backslash_as_literal() {
        let output_path = "target/rubash-read-sr-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("read -sr value <<< 'alpha\\ beta'; printf '<%s>' \"$value\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "<alpha\\ beta>");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_read_multiple_names_respects_backslash_escaped_custom_ifs() {
        let output_path = "target/rubash-read-escaped-custom-ifs-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "IFS=, read first rest <<< 'alpha\\,beta,gamma'; printf '<%s><%s>\\n' \"$first\" \"$rest\" > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "<alpha,beta><gamma>\n"
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
    fn test_read_n_zero_succeeds_with_empty_value() {
        let output_path = "target/rubash-read-n-zero-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("read -n 0 value <<< abc; printf '<%s>:%s' \"$value\" $? > {output_path}");
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
        let input =
            format!("read -N 0 value <<< abc; printf '<%s>:%s' \"$value\" $? > {output_path}");
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
    fn test_read_array_n_zero_assigns_empty_array() {
        let output_path = "target/rubash-read-array-n-zero-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "read -a values -n 0 <<< abc; printf '%s:%s' \"${{#values[@]}}\" \"$?\" > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0:0");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_read_n_rejects_negative_count() {
        let output_path = "target/rubash-read-n-negative-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("read -n -1 value <<< abc; echo $? > {output_path}");
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
    fn test_read_compact_n_rejects_non_numeric_count() {
        let output_path = "target/rubash-read-n-invalid-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("read -nabc value <<< abc; echo $? > {output_path}");
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
    fn test_read_combined_rn_reads_raw_limited_characters() {
        let output_path = "target/rubash-read-rn-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("read -rn3 value <<< 'a\\bcdef'; printf '<%s>' \"$value\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "<a\\b>");
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
    fn test_read_combined_r_upper_n_reads_raw_through_newline() {
        let output_path = "target/rubash-read-r-upper-n-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("read -rN4 value <<< $'ab\\ncd'; printf '<%s>' \"$value\" > {output_path}");
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
    fn test_read_combined_rd_uses_delimiter_and_raw_input() {
        let output_path = "target/rubash-read-rd-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("read -rd / value <<< 'a\\ b/c'; printf '<%s>' \"$value\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "<a\\ b>");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_read_combined_rd_compact_uses_delimiter_and_raw_input() {
        let output_path = "target/rubash-read-rd-compact-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("read -rd/ value <<< 'a\\ b/c'; printf '<%s>' \"$value\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "<a\\ b>");
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
    fn test_read_compact_a_uses_attached_array_name() {
        let output_path = "target/rubash-read-a-attached-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("read -aarr <<< 'alpha beta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
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
    fn test_read_a_processes_backslash_escaped_whitespace() {
        let output_path = "target/rubash-read-a-backslash-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("read -a arr <<< 'a\\ b c'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}");
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
    fn test_read_a_processes_backslash_escaped_custom_ifs() {
        let output_path = "target/rubash-read-a-escaped-custom-ifs-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "IFS=, read -a arr <<< 'a\\,b,c'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
        );
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
    fn test_for_loop_exit_status_matches_body_or_zero_iterations() {
        let output_path = "target/rubash-for-status-output.txt";
        let _ = fs::remove_file(output_path);
        let loop_only_tokens = tokenize("for item in one; do false; done");
        let loop_only_ast = parse(&loop_only_tokens);
        let mut loop_only_executor = Executor::new();
        let loop_only_result = loop_only_executor.execute_ast(&loop_only_ast);
        assert!(loop_only_result.is_ok());
        assert_eq!(loop_only_executor.last_exit_code(), 1);

        let input = format!(
            "for item in one; do false; done; echo $? > {output_path}; false; for item in; do true; done; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_for_loop_executes() {
        let output_path = "target/rubash-arithmetic-for-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("for (( i = 0; i < 3; i++ )); do echo $i >> {output_path}; done");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        assert!(ast.commands[0]
            .for_command
            .as_ref()
            .and_then(|for_command| for_command.arithmetic.as_ref())
            .is_some());
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_for_loop_break_continue_and_empty_test() {
        let output_path = "target/rubash-arithmetic-for-control-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "for (( i = 0; ; i++ )); do if (( i == 1 )); then continue; fi; echo $i >> {output_path}; if (( i == 3 )); then break; fi; done"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n2\n3\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_for_loop_exit_status_matches_body_or_zero_iterations() {
        let output_path = "target/rubash-arithmetic-for-status-output.txt";
        let _ = fs::remove_file(output_path);
        let loop_only_tokens = tokenize("for (( i = 0; i < 1; i++ )); do false; done");
        let loop_only_ast = parse(&loop_only_tokens);
        let mut loop_only_executor = Executor::new();
        let loop_only_result = loop_only_executor.execute_ast(&loop_only_ast);
        assert!(loop_only_result.is_ok());
        assert_eq!(loop_only_executor.last_exit_code(), 1);

        let input = format!(
            "for (( i = 0; i < 1; i++ )); do false; done; echo $? > {output_path}; false; for (( i = 0; i < 0; i++ )); do true; done; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n");
        let _ = fs::remove_file(output_path);
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
    fn test_declare_capital_f_prints_function_names() {
        let output_path = "target/rubash-declare-F-output.txt";
        let status_path = "target/rubash-declare-F-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!(
            "foo() {{ echo hi; }}; bar() {{ :; }}; \
             declare -F foo missing bar > {output_path}; echo $? > {status_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "foo\nbar\n");
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_declare_capital_f_lists_all_functions() {
        let output_path = "target/rubash-declare-F-all-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("foo() {{ :; }}; bar() {{ :; }}; declare -F > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "declare -f bar\ndeclare -f foo\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_declare_lower_f_redirects_function_definition() {
        let output_path = "target/rubash-declare-f-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("foo() {{ echo hi; }}; declare -f foo > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "foo () \n{ \n    echo hi\n}\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_declare_exported_functions_are_empty_without_function_exports() {
        let output_path = "target/rubash-declare-exported-functions-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("foo() {{ :; }}; declare -xF > {output_path}; declare -xf >> {output_path}");
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
    fn test_export_f_marks_function_for_declare_export_listing() {
        let output_path = "target/rubash-export-f-output.txt";
        let status_path = "target/rubash-export-f-status.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
        let input = format!(
            "foo() {{ echo hi; }}; export -f foo; echo export:$? > {status_path}; \
             declare -xF > {output_path}; declare -xf >> {output_path}; export -pf >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "export:0\n");
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "declare -fx foo\ndeclare -fx foo\nfoo () \n{ \n    echo hi\n}\ndeclare -fx foo\nfoo () \n{ \n    echo hi\n}\n"
        );
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_hyphenated_function_name_can_be_called_and_exported() {
        let output_path = target_test_path("rubash-hyphen-function-output.txt");
        let _ = fs::remove_file(&output_path);
        let shell_output_path = shell_test_path(&output_path);
        let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
        let input = format!(
            "foo-a() {{ echo local; }}; foo-a > {shell_output_path}; \
             export -f foo-a; {rubash} -c foo-a >> {shell_output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "local\nlocal\n");
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_export_f_rejects_nonexportable_function_names() {
        let output_path = "target/rubash-export-nonexportable-function-output.txt";
        let error_path = "target/rubash-export-nonexportable-function-error.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
        let input = format!(
            "function foo=bar {{ echo equals; }}; foo\\=bar > {output_path}; \
             export -f 'foo=bar' 2> {error_path}; echo equals_status:$? >> {output_path}; \
             function /bin/echo {{ echo slash; }}; /bin/echo >> {output_path}; \
             export -f '/bin/echo' 2>> {error_path}; echo slash_status:$? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "equals\nequals_status:1\nslash\nslash_status:1\n"
        );
        let errors = fs::read_to_string(error_path).unwrap();
        assert!(errors.contains("export: foo=bar: cannot export"));
        assert!(errors.contains("export: /bin/echo: cannot export"));
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_export_nf_unmarks_exported_function() {
        let output_path = "target/rubash-export-nf-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("foo() {{ :; }}; export -f foo; export -nf foo; declare -xF > {output_path}");
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
    fn test_unset_f_clears_exported_function_attribute() {
        let output_path = "target/rubash-unset-exported-function-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "foo() {{ :; }}; export -f foo; unset -f foo; foo() {{ :; }}; declare -xF foo > {output_path}"
        );
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
    fn test_readonly_f_protects_function_from_unset_and_redefine() {
        let output_path = "target/rubash-readonly-function-output.txt";
        let error_path = "target/rubash-readonly-function-error.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
        let input = format!(
            "foo() {{ echo one; }}; readonly -f foo; echo readonly:$? > {output_path}; \
             unset -f foo 2> {error_path}; echo unset:$? >> {output_path}; \
             foo() {{ echo two; }}; echo redefine:$? >> {output_path}; foo >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "readonly:0\nunset:1\nredefine:1\none\n"
        );
        assert!(fs::read_to_string(error_path)
            .unwrap()
            .contains("unset: foo: cannot unset: readonly function"));
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_readonly_f_missing_function_reports_error() {
        let error_path = "target/rubash-readonly-missing-function-error.txt";
        let status_path = "target/rubash-readonly-missing-function-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("readonly -f missing 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        assert!(fs::read_to_string(error_path)
            .unwrap()
            .contains("readonly: missing: not a function"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_declare_fr_marks_function_readonly() {
        let output_path = "target/rubash-declare-readonly-function-output.txt";
        let error_path = "target/rubash-declare-readonly-function-error.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
        let input = format!(
            "foo() {{ echo one; }}; declare -fr foo; echo declare:$? > {output_path}; \
             unset -f foo 2> {error_path}; echo unset:$? >> {output_path}; \
             foo() {{ echo two; }}; echo redefine:$? >> {output_path}; foo >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "declare:0\nunset:1\nredefine:1\none\n"
        );
        assert!(fs::read_to_string(error_path)
            .unwrap()
            .contains("unset: foo: cannot unset: readonly function"));
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_declare_fx_exports_function_to_child_rubash() {
        let output_path = target_test_path("rubash-declare-export-function-child-output.txt");
        let _ = fs::remove_file(&output_path);
        let shell_output_path = shell_test_path(&output_path);
        let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
        let input = format!(
            "foo() {{ echo child; }}; declare -fx foo; {rubash} -c foo > {shell_output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "child\n");
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_declare_plus_fx_clears_exported_function_attribute() {
        let output_path = "target/rubash-declare-clear-export-function-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("foo() {{ :; }}; export -f foo; declare +fx foo; declare -xF > {output_path}");
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
    fn test_export_f_missing_function_reports_error() {
        let error_path = "target/rubash-export-f-missing-error.txt";
        let status_path = "target/rubash-export-f-missing-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("export -f missing 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        assert!(fs::read_to_string(error_path)
            .unwrap()
            .contains("export: missing: not a function"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_bash_func_environment_import_defines_function() {
        let output_path = "target/rubash-imported-function-output.txt";
        let _ = fs::remove_file(output_path);
        let old_value = std::env::var("BASH_FUNC_rubash_imported%%").ok();
        std::env::set_var("BASH_FUNC_rubash_imported%%", "() { echo imported; }");
        let input = format!("declare -F rubash_imported > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        match old_value {
            Some(value) => std::env::set_var("BASH_FUNC_rubash_imported%%", value),
            None => std::env::remove_var("BASH_FUNC_rubash_imported%%"),
        }

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "rubash_imported\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_export_f_makes_function_available_to_child_rubash() {
        let output_path = target_test_path("rubash-exported-function-child-output.txt");
        let _ = fs::remove_file(&output_path);
        let shell_output_path = shell_test_path(&output_path);
        let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
        let input = format!(
            "rubash_child_func() {{ echo child; }}; export -f rubash_child_func; \
             {rubash} -c rubash_child_func > {shell_output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "child\n");
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_function_call_redirects_entire_body_output() {
        let output_path = "target/rubash-function-call-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "rubash_redirect_func() {{ echo one; echo two; }}; \
             rubash_redirect_func > {output_path}; echo done >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "one\ntwo\ndone\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_function_call_redirects_stdin_to_body_reads() {
        let input_path = target_test_path("rubash-function-call-stdin-input.txt");
        let output_path = target_test_path("rubash-function-call-stdin-output.txt");
        let _ = fs::remove_file(&input_path);
        let _ = fs::remove_file(&output_path);
        fs::write(&input_path, "alpha\nbeta\n").unwrap();
        let shell_input_path = shell_test_path(&input_path);
        let shell_output_path = shell_test_path(&output_path);
        let input = format!(
            "rubash_stdin_func() {{ read first; read second; echo $first/$second; }}; \
             rubash_stdin_func < {shell_input_path} > {shell_output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "alpha/beta\n");
        let _ = fs::remove_file(&input_path);
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_local_outside_function_reports_error() {
        let error_path = "target/rubash-local-outside-error.txt";
        let status_path = "target/rubash-local-outside-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("local x=1 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        assert!(fs::read_to_string(error_path)
            .unwrap()
            .contains("local: can only be used in a function"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_local_assignment_is_restored_after_function() {
        let output_path = "target/rubash-local-restore-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "x=global; function f {{ local x=local; echo in:$x > {output_path}; }}; \
             f; echo out:$x >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "in:local\nout:global\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_local_new_variable_is_unset_after_function() {
        let output_path = "target/rubash-local-unset-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "function f {{ local -x RUBASH_LOCAL_TEMP=2; echo in:$RUBASH_LOCAL_TEMP > {output_path}; }}; \
             f; echo out:${{RUBASH_LOCAL_TEMP-unset}} >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "in:2\nout:unset\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_local_invalid_option_reports_usage() {
        let error_path = "target/rubash-local-invalid-error.txt";
        let status_path = "target/rubash-local-invalid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input =
            format!("function f {{ local -z 2> {error_path}; echo $? > {status_path}; }}; f");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("local: -z: invalid option"));
        assert!(error.contains("local: usage: local [option] name[=value] ..."));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_local_case_conversion_attributes_apply_to_assignments() {
        let output_path = "target/rubash-local-case-attrs-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "function f {{ local -u up=abc; echo up1:$up > {output_path}; up=def; echo up2:$up >> {output_path}; \
             local -l low=ABC; echo low1:$low >> {output_path}; low=XYZ; echo low2:$low >> {output_path}; }}; \
             f; echo out:${{up-unset}}:${{low-unset}} >> {output_path}; \
             up=abc; low=XYZ; echo after:$up:$low >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "up1:ABC\nup2:DEF\nlow1:abc\nlow2:xyz\nout:unset:unset\nafter:abc:XYZ\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_local_plus_attributes_clear_conversion_and_integer_flags() {
        let output_path = "target/rubash-local-plus-attrs-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "function f {{ local -u up=abc; local +u up; up=def; echo up:$up > {output_path}; \
             local -i n=2+3; local +i n; n=2+3; echo n:$n >> {output_path}; }}; f"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "up:def\nn:2+3\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_getopts_parses_explicit_option_argument() {
        let output_path = "target/rubash-getopts-explicit-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("getopts a: store -a aoptval; echo $?:$store:$OPTARG:$OPTIND > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0:a:aoptval:3\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_getopts_walks_clustered_options() {
        let output_path = "target/rubash-getopts-cluster-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "getopts ab opt -ab; echo $opt:$OPTIND:$? > {output_path}; getopts ab opt -ab; echo $opt:$OPTIND:$? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "a:1:0\nb:2:0\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_getopts_reports_missing_argument_modes() {
        let output_path = "target/rubash-getopts-missing-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "OPTERR=0; getopts a: opt -a; echo $?:$opt:${{OPTARG-unset}}:$OPTIND > {output_path}; \
             OPTIND=1; getopts :a: opt -a; echo $?:$opt:$OPTARG:$OPTIND >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0:?:unset:2\n0:::a:2\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_getopts_invalid_builtin_option_redirects_stderr() {
        let error_path = "target/rubash-getopts-invalid-error.txt";
        let status_path = "target/rubash-getopts-invalid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
        let input = format!("getopts -a opts name 2> {error_path}; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("getopts: -a: invalid option\n"));
        assert!(error.contains("getopts: usage: getopts optstring name [arg ...]\n"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_builtin_and_command_getopts_update_shell_state() {
        let output_path = "target/rubash-getopts-command-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "builtin getopts a opt -a; echo builtin:$?:$opt:$OPTIND > {output_path}; \
             OPTIND=1; command getopts a opt -a; echo command:$?:$opt:$OPTIND >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "builtin:0:a:2\ncommand:0:a:2\n"
        );
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
    fn test_shift_invalid_counts_redirect_stderr() {
        let error_path = "target/rubash-shift-invalid-stderr.txt";
        let output_path = "target/rubash-shift-invalid-status.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(output_path);
        let input = format!(
            "function s {{ shift x 2> {error_path}; echo nonnumeric:$?:$#:$1 > {output_path}; \
             shift -1 2>> {error_path}; echo negative:$?:$#:$1 >> {output_path}; }}; s one two"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "nonnumeric:1:2:one\nnegative:1:2:one\n"
        );
        let error = fs::read_to_string(error_path).unwrap();
        assert!(error.contains("shift: x: numeric argument required"));
        assert!(error.contains("shift: -1: shift count out of range"));
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_shift_too_many_arguments_fails_without_changing_positional_params() {
        let error_path = "target/rubash-shift-too-many-args-stderr.txt";
        let output_path = "target/rubash-shift-too-many-args-output.txt";
        let _ = fs::remove_file(error_path);
        let _ = fs::remove_file(output_path);
        let input = format!(
            "function s {{ shift 1 2 2> {error_path}; echo $? $# $1 > {output_path}; }}; s one two"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1 2 one\n");
        assert!(fs::read_to_string(error_path)
            .unwrap()
            .contains("shift: too many arguments"));
        let _ = fs::remove_file(error_path);
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
    fn test_shell_flags_expand_inside_words() {
        let output_path = "target/rubash-shell-flags-embedded-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("set -u; printf '%s\\n' \"flags:$-\" > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("flags:"));
        assert!(output.trim_end().contains('u'));
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
    fn test_set_nounset_updates_shell_flags_and_option_tests() {
        let output_path = "target/rubash-set-nounset-flags-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "set -u; echo $- > {output_path}; [[ -o nounset ]]; echo $? >> {output_path}; \
             set +u; echo $- >> {output_path}; [[ -o nounset ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let lines: Vec<String> = fs::read_to_string(output_path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert!(lines[0].contains('u'));
        assert_eq!(lines[1], "0");
        assert!(!lines[2].contains('u'));
        assert_eq!(lines[3], "1");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_set_nounset_with_positional_operands() {
        let output_path = "target/rubash-set-nounset-operands-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("name=beta; set -u alpha $name; echo $# $1 $2 $- > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("2 alpha beta "));
        assert!(output.trim_end().ends_with('u'));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_set_o_nounset_updates_shell_flags() {
        let output_path = "target/rubash-set-o-nounset-flags-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("set -o nounset; echo $- > {output_path}; test -o nounset");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains('u'));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_shellopts_reflects_set_options() {
        let output_path = "target/rubash-shellopts-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "echo $SHELLOPTS > {output_path}; shopt -so physical; echo $SHELLOPTS >> {output_path}; readonly -p >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("braceexpand:hashall:interactive-comments\n"));
        assert!(output.contains("braceexpand:hashall:interactive-comments:physical\n"));
        assert!(output.contains(
            "declare -r SHELLOPTS=\"braceexpand:hashall:interactive-comments:physical\""
        ));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_set_posix_updates_visible_option_state() {
        let output_path = "target/rubash-set-posix-option-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "set -o posix; type break > {output_path}; set -o >> {output_path}; \
             set +o posix; type break >> {output_path}; set -o >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("break is a special shell builtin\n"));
        assert!(output.contains("break is a shell builtin\n"));
        let posix_lines: Vec<_> = output
            .lines()
            .filter(|line| line.starts_with("posix"))
            .collect();
        assert_eq!(posix_lines, ["posix          \ton", "posix          \toff"]);
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_shellopts_assignment_reports_readonly() {
        let status_path = "target/rubash-shellopts-readonly-status.txt";
        let _ = fs::remove_file(status_path);
        let input = format!("SHELLOPTS=ignored; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_bashopts_reflects_shopt_options() {
        let output_path = "target/rubash-bashopts-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "echo $BASHOPTS > {output_path}; \
             shopt -s checkhash; echo $BASHOPTS >> {output_path}; \
             shopt -u checkwinsize; echo $BASHOPTS >> {output_path}; \
             shopt -u checkhash; \
             readonly -p >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert!(lines[0].contains("checkwinsize"));
        assert!(lines[0].contains("cmdhist"));
        assert!(!lines[0].contains("checkhash"));
        assert!(lines[1].contains("checkhash"));
        assert!(lines[1].contains("checkwinsize"));
        assert!(lines[2].contains("checkhash"));
        assert!(!lines[2].contains("checkwinsize"));
        assert!(output.contains("declare -r BASHOPTS=\""));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_bashopts_assignment_reports_readonly() {
        let status_path = "target/rubash-bashopts-readonly-status.txt";
        let _ = fs::remove_file(status_path);
        let input = format!("BASHOPTS=$BASHOPTS; echo $? > {status_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
        let _ = fs::remove_file(status_path);
    }

    #[test]
    fn test_set_noclobber_updates_shell_flags() {
        let output_path = "target/rubash-set-noclobber-flags-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("set -C; echo $- > {output_path}; set +C; echo $- >> {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let lines: Vec<String> = fs::read_to_string(output_path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert!(lines[0].contains('C'));
        assert!(!lines[1].contains('C'));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_set_noglob_updates_shell_flags_and_option_tests() {
        let output_path = "target/rubash-set-noglob-flags-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "set -f; echo $- > {output_path}; [[ -o noglob ]]; echo $? >> {output_path}; \
             set +f; echo $- >> {output_path}; [[ -o noglob ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let lines: Vec<String> = fs::read_to_string(output_path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert!(lines[0].contains('f'));
        assert_eq!(lines[1], "0");
        assert!(!lines[2].contains('f'));
        assert_eq!(lines[3], "1");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_set_noglob_with_positional_operands() {
        let output_path = "target/rubash-set-noglob-operands-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("set -f alpha beta; echo $# $1 $2 $- > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.starts_with("2 alpha beta "));
        assert!(output.trim_end().ends_with('f'));
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_set_noexec_updates_shell_option() {
        let tokens = tokenize("set -n");
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(executor.get_env("__RUBASH_SETOPT_noexec"), Some("1"));
    }

    #[test]
    fn test_set_noexec_skips_later_commands_and_redirections() {
        let output_path = "target/rubash-noexec-skips-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("set -n; echo should-not-run > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert!(!std::path::Path::new(output_path).exists());
    }

    #[test]
    fn test_additional_set_short_flags_update_shell_options() {
        let output_path = "target/rubash-set-extra-flags-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "echo $- > {output_path}; set -abPkv; echo $- >> {output_path}; \
             [[ -o allexport ]]; echo $? >> {output_path}; [[ -o notify ]]; echo $? >> {output_path}; \
             [[ -o physical ]]; echo $? >> {output_path}; [[ -o keyword ]]; echo $? >> {output_path}; \
             [[ -o verbose ]]; echo $? >> {output_path}; set +abPkvh; echo $- >> {output_path}; \
             [[ -o hashall ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let lines: Vec<String> = fs::read_to_string(output_path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert!(lines[0].contains('h'));
        for flag in ['a', 'b', 'P', 'k', 'v'] {
            assert!(lines[1].contains(flag));
        }
        assert_eq!(lines[2..7], ["0", "0", "0", "0", "0"].map(str::to_string));
        for flag in ['a', 'b', 'P', 'k', 'v', 'h'] {
            assert!(!lines[7].contains(flag));
        }
        assert_eq!(lines[8], "1");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_remaining_stateful_set_short_flags_update_shell_options() {
        let output_path = "target/rubash-set-remaining-flags-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "echo $- > {output_path}; \
             [[ -o privileged ]]; echo $? >> {output_path}; \
             set -EHTpt; echo $- >> {output_path}; \
             [[ -o errtrace ]]; echo $? >> {output_path}; \
             [[ -o histexpand ]]; echo $? >> {output_path}; \
             [[ -o functrace ]]; echo $? >> {output_path}; \
             [[ -o privileged ]]; echo $? >> {output_path}; \
             [[ -o onecmd ]]; echo $? >> {output_path}; \
             set +BEHTpt; echo $- >> {output_path}; \
             [[ -o braceexpand ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        let lines: Vec<String> = fs::read_to_string(output_path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert!(lines[0].contains('B'));
        for flag in ['E', 'H', 'T', 'p', 't'] {
            assert!(!lines[0].contains(flag));
        }
        assert_eq!(lines[1], "1");
        for flag in ['E', 'H', 'T', 'p', 't'] {
            assert!(lines[2].contains(flag));
        }
        assert_eq!(lines[3..8], ["0", "0", "0", "0", "0"].map(str::to_string));
        for flag in ['B', 'E', 'H', 'T', 'p', 't'] {
            assert!(!lines[8].contains(flag));
        }
        assert_eq!(lines[9], "1");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_noclobber_prevents_output_overwrite() {
        let output_path = "target/rubash-noclobber-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "old\n").unwrap();
        let input = format!("set -C; echo new > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::IoError(_))));
        assert_eq!(fs::read_to_string(output_path).unwrap(), "old\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_noclobber_can_be_disabled_for_output_overwrite() {
        let output_path = "target/rubash-noclobber-disabled-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "old\n").unwrap();
        let input = format!("set -C; set +C; printf new > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "new");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_clobber_redirect_overrides_noclobber() {
        let output_path = "target/rubash-clobber-redirect-output.txt";
        let _ = fs::remove_file(output_path);
        fs::write(output_path, "old\n").unwrap();
        let input = format!("set -C; echo new >| {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "new\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_noclobber_prevents_stderr_overwrite() {
        let error_path = "target/rubash-noclobber-stderr.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "old\n").unwrap();
        let input = format!("set -C; unalias no_such_alias 2> {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::IoError(_))));
        assert_eq!(fs::read_to_string(error_path).unwrap(), "old\n");
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_stderr_clobber_redirect_overrides_noclobber() {
        let error_path = "target/rubash-clobber-stderr.txt";
        let _ = fs::remove_file(error_path);
        fs::write(error_path, "old\n").unwrap();
        let input = format!("set -C; unalias no_such_alias 2>| {error_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 1);
        let output = fs::read_to_string(error_path).unwrap();
        assert!(output.contains("no_such_alias"));
        assert!(!output.contains("old"));
        let _ = fs::remove_file(error_path);
    }

    #[test]
    fn test_nounset_errors_for_unbound_variable() {
        let output_path = "target/rubash-nounset-unbound-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset RUBASH_NOUNSET_MISSING; set -u; echo $RUBASH_NOUNSET_MISSING > {output_path}; echo after > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(127))));
        assert_eq!(executor.last_exit_code(), 127);
        assert!(!std::path::Path::new(output_path).exists());
    }

    #[test]
    fn test_nounset_errors_for_unbound_positional_parameter() {
        let output_path = "target/rubash-nounset-positional-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("set -u; echo $1 > {output_path}; echo after > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(127))));
        assert_eq!(executor.last_exit_code(), 127);
        assert!(!std::path::Path::new(output_path).exists());
    }

    #[test]
    fn test_nounset_allows_default_parameter_expansion() {
        let output_path = "target/rubash-nounset-default-output.txt";
        let _ = fs::remove_file(output_path);
        let input =
            format!("unset RUBASH_NOUNSET_DEFAULT; set -u; echo ${{RUBASH_NOUNSET_DEFAULT:-fallback}} > {output_path}");
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
    fn test_nounset_errors_for_unbound_assignment_value() {
        let output_path = "target/rubash-nounset-assignment-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset RUBASH_NOUNSET_ASSIGNMENT; set -u; value=$RUBASH_NOUNSET_ASSIGNMENT; echo after > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(127))));
        assert_eq!(executor.last_exit_code(), 127);
        assert!(!std::path::Path::new(output_path).exists());
    }

    #[test]
    fn test_nounset_errors_for_unbound_assignment_prefix() {
        let output_path = "target/rubash-nounset-assignment-prefix-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "unset RUBASH_NOUNSET_PREFIX; set -u; value=$RUBASH_NOUNSET_PREFIX echo after > {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(127))));
        assert_eq!(executor.last_exit_code(), 127);
        assert!(!std::path::Path::new(output_path).exists());
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
    fn test_conditional_shell_option_unary() {
        let output_path = "target/rubash-conditional-shell-option-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "set -o errexit; [[ -o errexit ]]; echo $? > {output_path}; set +o errexit; [[ -o errexit ]]; echo $? >> {output_path}; [[ -o no_such_option ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_short_errexit_updates_shell_option() {
        let output_path = "target/rubash-short-errexit-option-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "set -e; [[ -o errexit ]]; echo $? > {output_path}; set +e; [[ -o errexit ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_set_o_errexit_exits_on_failed_command() {
        let output_path = "target/rubash-set-o-errexit-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!("set -o errexit; false; echo after > {output_path}");
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
        assert_eq!(executor.last_exit_code(), 1);
        assert!(!std::path::Path::new(output_path).exists());
    }

    #[test]
    fn test_test_shell_option_unary() {
        let output_path = "target/rubash-test-shell-option-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "set -o errexit; test -o errexit; echo $? > {output_path}; set +o errexit; test -o errexit; echo $? >> {output_path}; [ -o no_such_option ]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n1\n");
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
    fn test_arithmetic_expansion_evaluates_expressions() {
        let output_path = "target/rubash-arithmetic-expansion-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=5; echo $((n+2*3)) > {output_path}; echo $((16#ff-250)) >> {output_path}; echo $((n>4?7:9)) >> {output_path}; echo $((2**3**2)) >> {output_path}; echo pre$((1+(2*3)))post >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "11\n5\n7\n512\npre7post\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_expansion_applies_side_effects() {
        let output_path = "target/rubash-arithmetic-expansion-side-effects-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=1; echo $((n++)) $n > {output_path}; echo pre$((n+=4))post $n >> {output_path}; echo $((a=b=3)) $a $b >> {output_path}; x=$((n++)); echo $x $n >> {output_path}; echo $((0 && (n+=99))) $n >> {output_path}; echo $((1 || (n+=99))) $n >> {output_path}; echo $((1 && (n+=2))) $n >> {output_path}; echo $((0 || (n+=3))) $n >> {output_path}; echo $((1 ? (n+=4) : (n+=99))) $n >> {output_path}; echo $((0 ? (n+=99) : (n+=5))) $n >> {output_path}; echo $((n++ + 1)) $n >> {output_path}; echo $((++n + 1)) $n >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "1 2\npre6post 6\n3 3 3\n6 7\n0 7\n1 7\n1 9\n1 12\n16 16\n21 21\n22 22\n24 23\n"
        );
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
    fn test_arithmetic_assignments_evaluate_rhs_recursively() {
        let output_path = "target/rubash-arithmetic-recursive-assign-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "a=0; b=0; (( a = b = 3 )); echo $? $a $b > {output_path}; let 'a+=b=4'; echo $? $a $b >> {output_path}; (( z = a = b = 0 )); echo $? $a $b $z >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0 3 3\n0 7 4\n1 0 0 0\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_variables_evaluate_recursively() {
        let output_path = "target/rubash-arithmetic-recursive-vars-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "x='1+2'; echo $((x)) > {output_path}; x=y; y=5; echo $((x)) >> {output_path}; n=1; x='n+=2'; echo $((x)) $n >> {output_path}; n=1; x='n+=2'; [[ x -eq 3 ]]; echo $? $n >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "3\n5\n3 3\n0 3\n");
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_array_subscripts_read_and_update() {
        let output_path = "target/rubash-arithmetic-array-subscript-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "arr=(4 7); i=1; echo $((arr[i]+1)) > {output_path}; (( arr[i] += 2 )); echo ${{arr[1]}} $((arr[i])) >> {output_path}; (( arr[0]++ )); echo ${{arr[0]}} >> {output_path}; echo $((arr[2])) >> {output_path}; arr[2]=5; echo $((arr[2])) >> {output_path}; i=0; echo $((arr[i++])) $i >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "8\n9 9\n5\n0\n5\n5 1\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_assoc_subscripts_read_and_update() {
        let output_path = "target/rubash-arithmetic-assoc-subscript-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "declare -A assoc; key=one; assoc[one]=7; echo $((assoc[one]+1)) > {output_path}; echo $((assoc[key]+1)) >> {output_path}; (( assoc[key] += 2 )); echo ${{assoc[one]}} ${{assoc[key]}} $((assoc[key])) >> {output_path}; (( assoc[two]++ )); echo ${{assoc[two]}} >> {output_path}; echo $((assoc[missing])) >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "8\n1\n7 2 2\n1\n0\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_grouped_arithmetic_assignments_have_side_effects() {
        let output_path = "target/rubash-arithmetic-grouped-assign-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=0; (( (n = 3) )); echo $? $n > {output_path}; (( ((m = 0)) )); echo $? $m >> {output_path}; (( (1 ? 4 : 1 / 0) )); echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0 3\n1 0\n0\n");
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
    fn test_integer_variables_evaluate_assignments_as_arithmetic() {
        let output_path = "target/rubash-integer-assignment-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "declare -i n=2+3*4; echo $n > {output_path}; n=2**3; echo $n >> {output_path}; n+=2*5; echo $n >> {output_path}; declare -p n >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "14\n8\n18\ndeclare -i n=\"18\"\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_case_conversion_variables_transform_assignments() {
        let output_path = "target/rubash-case-conversion-vars-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "declare -u U=abc; echo $U > {output_path}; U=def; echo $U >> {output_path}; \
             declare -l L=ABC; echo $L >> {output_path}; L=XYZ; echo $L >> {output_path}; \
             declare -p U L >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "ABC\nDEF\nabc\nxyz\ndeclare -u U=\"DEF\"\ndeclare -l L=\"xyz\"\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_declare_plus_attributes_clear_variable_flags() {
        let output_path = "target/rubash-declare-plus-attrs-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "declare -u U=abc; declare +u U; U=ghi; echo $U > {output_path}; declare -p U >> {output_path}; \
             declare -l L=ABC; declare +l L; L=XYZ; echo $L >> {output_path}; declare -p L >> {output_path}; \
             declare -i n=2+3; declare +i n; n=2+3; echo $n >> {output_path}; declare -p n >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "ghi\ndeclare -- U=\"ghi\"\nXYZ\ndeclare -- L=\"XYZ\"\n2+3\ndeclare -- n=\"2+3\"\n"
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
    fn test_let_bitwise_assignment_operators() {
        let output_path = "target/rubash-let-bitwise-assign-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=12; let 'n&=10'; echo $? $n > {output_path}; let 'n|=1'; echo $? $n >> {output_path}; let 'n^=3'; echo $? $n >> {output_path}; let 'n<<=2'; echo $? $n >> {output_path}; let 'n>>=1'; echo $? $n >> {output_path}; let 'n&=0'; echo $? $n >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0 8\n0 9\n0 10\n0 40\n0 20\n1 0\n"
        );
        let _ = fs::remove_file(output_path);
    }

    #[test]
    fn test_arithmetic_command_bitwise_assignment_operators() {
        let output_path = "target/rubash-arithmetic-command-bitwise-assign-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "n=12; (( n &= 10 )); echo $? $n > {output_path}; (( n |= 1 )); echo $? $n >> {output_path}; (( n ^= 3 )); echo $? $n >> {output_path}; (( n <<= 2 )); echo $? $n >> {output_path}; (( n >>= 1 )); echo $? $n >> {output_path}; (( n &= 0 )); echo $? $n >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0 8\n0 9\n0 10\n0 40\n0 20\n1 0\n"
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
    fn test_arithmetic_conditional_operator_is_lazy() {
        let output_path = "target/rubash-arithmetic-conditional-lazy-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "(( 1 ? 4 : 1 / 0 )); echo $? > {output_path}; (( 0 ? 1 / 0 : 4 )); echo $? >> {output_path}; (( 1 ? 0 : 1 / 0 )); echo $? >> {output_path}; (( 0 ? 1 / 0 : 0 )); echo $? >> {output_path}; (( 1 ? 1 ? 5 : 1 / 0 : 1 / 0 )); echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n0\n1\n1\n0\n");
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
            "value=abc123; pattern='([a-z]+)([0-9]+)'; [[ $value =~ $pattern ]]; echo $? ${{BASH_REMATCH[0]}} ${{BASH_REMATCH[1]}} ${{BASH_REMATCH[2]}} > {output_path}; [[ $value =~ z+ ]]; echo $? >> {output_path}; [[ $value =~ '[' ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "0 abc123 abc 123\n1\n2\n"
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
    fn test_conditional_file_binary_checks_paths() {
        let output_path = "target/rubash-conditional-file-binary-output.txt";
        let older_path = "target/rubash-conditional-file-binary-older.txt";
        let newer_path = "target/rubash-conditional-file-binary-newer.txt";
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(older_path);
        let _ = fs::remove_file(newer_path);
        fs::write(older_path, "old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(25));
        fs::write(newer_path, "new").unwrap();
        let input = format!(
            "[[ {newer_path} -nt {older_path} ]]; echo $? > {output_path}; [[ {older_path} -ot {newer_path} ]]; echo $? >> {output_path}; [[ {older_path} -ef {older_path} ]]; echo $? >> {output_path}; test {newer_path} -nt {older_path}; echo $? >> {output_path}; [[ {older_path} -nt {newer_path} ]]; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n0\n0\n0\n1\n");
        let _ = fs::remove_file(output_path);
        let _ = fs::remove_file(older_path);
        let _ = fs::remove_file(newer_path);
    }

    #[test]
    fn test_test_builtin_parenthesizes_logical_expressions() {
        let output_path = "target/rubash-test-parentheses-output.txt";
        let _ = fs::remove_file(output_path);
        let input = format!(
            "test \\( x = x -o '' \\) -a ''; echo $? > {output_path}; test \\( x = y -o ok = ok \\) -a yes = yes; echo $? >> {output_path}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(result.is_ok());
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n");
        let _ = fs::remove_file(output_path);
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
