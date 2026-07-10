use super::super::*;
use std::fs;

#[test]
fn test_brace_group_redirects_combined_stdout() {
    let output_path = target_test_path("rubash-brace-group-redirect-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input =
        format!("{{ echo alpha; echo beta; }} > {shell_output_path}; cat {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "alpha\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_brace_group_input_redirect_feeds_body_reads() {
    let input_path = target_test_path("rubash-brace-group-input.txt");
    let output_path = target_test_path("rubash-brace-group-input-output.txt");
    let shell_input_path = shell_test_path(&input_path);
    let shell_output_path = shell_test_path(&output_path);
    fs::write(&input_path, "alpha\nbeta\n").unwrap();
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "{{ read first; read second; }} < {shell_input_path}; printf '%s/%s\\n' \"$first\" \"$second\" > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "alpha/beta\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_brace_group_here_string_feeds_body_read() {
    let output_path = target_test_path("rubash-brace-group-herestring-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input =
        format!("{{ read value; echo got:$value; }} <<< alpha > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "got:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_subshell_input_redirect_feeds_body_reads() {
    let input_path = target_test_path("rubash-subshell-input.txt");
    let output_path = target_test_path("rubash-subshell-input-output.txt");
    let shell_input_path = shell_test_path(&input_path);
    let shell_output_path = shell_test_path(&output_path);
    fs::write(&input_path, "alpha\nbeta\n").unwrap();
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "( read first; read second; printf '%s/%s\\n' \"$first\" \"$second\" ) < {shell_input_path} > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "alpha/beta\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_subshell_here_string_feeds_body_read() {
    let output_path = target_test_path("rubash-subshell-herestring-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!("( read value; echo got:$value ) <<< alpha > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "got:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_brace_group_preserves_quoted_bracket_words() {
    let input = "[ $# -lt 1 ] && {\n\
         echo \"zprintf: usage: zprintf format [args ...]\" >&2\n\
         exit 2\n\
    }\n\
    echo after";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(2))));
    assert_eq!(executor.last_exit_code(), 2);
}

#[test]
fn test_command_substitution_output_is_not_reexpanded() {
    let output_path = "target/rubash-command-substitution-no-reexpand-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() {{ value=$(echo $2 | sed 's/\\!\\*/\"$\\@\"/g'); printf '<%s>\\n' \"$value\" > {output_path}; }}; f star 'echo !*'"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<echo \"$@\">\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_v_pipeline_command_substitution_outputs_description() {
    let output_path = "target/rubash-command-v-pipeline-comsub-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("value=$(command -v echo | sort -u); printf '<%s>\\n' \"$value\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<echo>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_command_substitution_sed_replaces_backslashes() {
    let output_path = "target/rubash-comsub-sed-backslash-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=$(printf '%s\\n' 'a\\\\b' | sed 's#\\\\\\\\#/#g'); printf '<%s>\\n' \"$value\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<a/b>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_escaped_quotes_survive_adjacent_command_substitution() {
    let output_path = "target/rubash-command-substitution-escaped-quote-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "echo alias hi=\\'$(echo \"echo hello\" | sed \"s:':'\\\\\\\\'':g\")\\' > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alias hi='echo hello'\n"
    );
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
