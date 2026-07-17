use super::super::*;
use std::fs;

fn assert_pid_text(value: &str) {
    let pid = value.parse::<u32>().expect("background pid is numeric");
    assert!(pid > 0);
}

fn assert_status_bang_line(line: &str, prefix: &str) {
    let pid = line.strip_prefix(prefix).expect("status/bang prefix");
    assert_pid_text(pid);
}

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
    let input = format!("{{ read value; echo got:$value; }} <<< alpha > {shell_output_path}");
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
fn test_brace_group_stdout_fd_copy_does_not_create_literal_target() {
    let literal_fd_path = "&2";
    let _ = fs::remove_file(literal_fd_path);
    let tokens = tokenize("{ echo grouped; } >&2");
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(fs::metadata(literal_fd_path).is_err());
    let _ = fs::remove_file(literal_fd_path);
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
    let output = fs::read_to_string(output_path).unwrap();
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "<>");
    assert_status_bang_line(lines[1], "status:0 bang:");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_last_background_pid_parameter_tracks_background_compound_command() {
    let output_path = "target/rubash-last-background-compound-pid-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "{{ echo grouped > {output_path}; }} & pid=$!; launch=$?; wait \"$pid\"; printf 'status:%s bang:%s\\n' \"$launch\" \"$pid\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].background_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "grouped");
    assert_status_bang_line(lines[1], "status:0 bang:");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_background_if_command_executes_and_updates_bang_pid() {
    let output_path = "target/rubash-background-if-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "if true; then echo if-body > {output_path}; fi & \
         pid=$!; launch=$?; wait \"$pid\"; printf 'status:%s bang:%s\\n' \"$launch\" \"$pid\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].background_command.is_some());
    assert!(ast.commands[0]
        .background_command
        .as_ref()
        .unwrap()
        .command
        .if_command
        .is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "if-body");
    assert_status_bang_line(lines[1], "status:0 bang:");
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
fn test_background_function_definition_does_not_leak_to_parent() {
    let output_path = "target/rubash-background-function-definition-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "bgf() {{ echo function-call > {output_path}; }} & \
         pid=$!; launch=$?; wait \"$pid\"; bgf 2>/dev/null; \
         printf 'status:%s call:%s bang:%s\\n' \"$launch\" \"$?\" \"$pid\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].background_command.is_some());
    assert!(ast.commands[0]
        .background_command
        .as_ref()
        .unwrap()
        .command
        .function_command
        .is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let line = output.trim_end();
    let rest = line
        .strip_prefix("status:0 call:")
        .expect("function definition status line");
    let (call_status, pid) = rest.split_once(" bang:").unwrap();
    assert_ne!(call_status, "0");
    assert_pid_text(pid);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_background_time_prefix_updates_bang_pid() {
    let output_path = "target/rubash-background-time-prefix-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("time -p true & printf 'status:%s bang:%s\\n' \"$?\" \"$!\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let background = ast.commands[0].background_command.as_ref().unwrap();
    assert!(background.command.time_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert_status_bang_line(output.trim_end(), "status:0 bang:");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_background_case_command_executes_and_updates_bang_pid() {
    let output_path = "target/rubash-background-case-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "case yes in yes) echo yes > {output_path} ;; *) echo no > {output_path} ;; esac & \
         pid=$!; launch=$?; wait \"$pid\"; printf 'status:%s bang:%s\\n' \"$launch\" \"$pid\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let background = ast.commands[0].background_command.as_ref().unwrap();
    assert!(background.command.case_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "yes");
    assert_status_bang_line(lines[1], "status:0 bang:");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_background_loop_commands_execute_and_update_bang_pid() {
    let output_path = "target/rubash-background-loop-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "i=0; while (( i < 1 )); do echo while >> {output_path}; ((i++)); done & \
         pid=$!; launch=$?; wait \"$pid\"; printf 'while-status:%s bang:%s\\n' \"$launch\" \"$pid\" >> {output_path}; \
         until true; do echo never >> {output_path}; done & \
         pid=$!; launch=$?; wait \"$pid\"; printf 'until-status:%s bang:%s\\n' \"$launch\" \"$pid\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(
        ast.commands
            .iter()
            .filter(|command| command
                .background_command
                .as_ref()
                .is_some_and(|background| background.command.loop_command.is_some()))
            .count(),
        2
    );
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "while");
    assert_status_bang_line(lines[1], "while-status:0 bang:");
    assert_status_bang_line(lines[2], "until-status:0 bang:");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_background_iteration_commands_execute_and_update_bang_pid() {
    let output_path = "target/rubash-background-iteration-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for value in one two; do echo for:$value >> {output_path}; done & \
         pid=$!; launch=$?; wait \"$pid\"; printf 'for-status:%s bang:%s\\n' \"$launch\" \"$pid\" >> {output_path}; \
         select value in one two; do echo select:$value >> {output_path}; break; done <<< 2 & \
         pid=$!; launch=$?; wait \"$pid\"; printf 'select-status:%s bang:%s\\n' \"$launch\" \"$pid\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands.iter().any(|command| command
        .background_command
        .as_ref()
        .is_some_and(|background| background.command.for_command.is_some())));
    assert!(ast.commands.iter().any(|command| command
        .background_command
        .as_ref()
        .is_some_and(|background| background.command.select_command.is_some())));
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "for:one");
    assert_eq!(lines[1], "for:two");
    assert_status_bang_line(lines[2], "for-status:0 bang:");
    assert_eq!(lines[3], "select:two");
    assert_status_bang_line(lines[4], "select-status:0 bang:");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_background_test_commands_execute_and_update_bang_pid() {
    let output_path = "target/rubash-background-test-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "count=0; (( count += 1 )) & \
         pid=$!; launch=$?; wait \"$pid\"; printf 'arith-status:%s count:%s bang:%s\\n' \"$launch\" \"$count\" \"$pid\" > {output_path}; \
         value=yes; [[ $value == yes ]] & \
         pid=$!; launch=$?; wait \"$pid\"; printf 'cond-status:%s bang:%s\\n' \"$launch\" \"$pid\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands.iter().any(|command| command
        .background_command
        .as_ref()
        .is_some_and(|background| background.command.arithmetic_command.is_some())));
    assert!(ast.commands.iter().any(|command| command
        .background_command
        .as_ref()
        .is_some_and(|background| background.command.conditional_command.is_some())));
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let lines = output.lines().collect::<Vec<_>>();
    assert_status_bang_line(lines[0], "arith-status:0 count:0 bang:");
    assert_status_bang_line(lines[1], "cond-status:0 bang:");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_background_pipeline_executes_and_waits_for_child_status() {
    let output_path = "target/rubash-background-pipeline-output.txt";
    let status_path = "target/rubash-background-pipeline-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "printf 'a\\nb\\n' | wc -l > {output_path} & \
         pid=$!; launch=$?; wait \"$pid\"; \
         printf 'launch:%s wait:%s bang:%s\\n' \"$launch\" \"$?\" \"$pid\" > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let background = ast.commands[0].background_command.as_ref().unwrap();
    assert!(background.command.pipeline_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2\n");
    let status = fs::read_to_string(status_path).unwrap();
    assert_status_bang_line(status.trim_end(), "launch:0 wait:0 bang:");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
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
