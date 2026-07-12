use super::super::*;
use std::fs;

#[test]
fn test_select_command_redirects_body_stdout() {
    let output_path = "target/rubash-select-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "select item in alpha beta; do echo chosen:$item; break; done <<< '2' > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].select_command.is_some());
    assert!(ast.commands[0].here_string.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_command_accepts_brace_group_body() {
    let output_path = "target/rubash-select-brace-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "select item in alpha beta; {{ echo chosen:$item; break; }} <<< '2' > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].select_command.is_some());
    assert!(ast.commands[0].here_string.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_command_redirect_creates_file_for_empty_word_list() {
    let output_path = "target/rubash-select-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("select item in; do echo bad; done > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].select_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_command_input_redirect_feeds_choice() {
    let input_path = "target/rubash-select-command-input.txt";
    let output_path = "target/rubash-select-command-input-output.txt";
    fs::write(input_path, "2\n").unwrap();
    let _ = fs::remove_file(output_path);
    let input = format!(
        "select item in alpha beta; do echo chosen:$item; break; done < {input_path} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].select_command.is_some());
    assert!(ast.commands[0].redirect_in.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(input_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_without_in_uses_positional_params() {
    let output_path = "target/rubash-select-default-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("select item; do echo chosen:$item; break; done <<< '2' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let select = ast.commands[0].select_command.as_ref().unwrap();
    assert!(select.default_positional);
    let mut executor = Executor::new();
    executor.set_positional_params(vec!["alpha".to_string(), "beta".to_string()]);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_brace_body_without_in_uses_positional_params() {
    let output_path = "target/rubash-select-brace-body-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("select item; {{ echo chosen:$item; break; }} <<< '2' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let select = ast.commands[0].select_command.as_ref().unwrap();
    assert!(select.default_positional);
    let mut executor = Executor::new();
    executor.set_positional_params(vec!["alpha".to_string(), "beta".to_string()]);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_select_explicit_empty_in_does_not_use_positional_params() {
    let output_path = "target/rubash-select-empty-in-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("select item in; do echo bad; done <<< '1' > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let select = ast.commands[0].select_command.as_ref().unwrap();
    assert!(!select.default_positional);
    let mut executor = Executor::new();
    executor.set_positional_params(vec!["alpha".to_string()]);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_select_command_executes_choice() {
    let output_path = "target/rubash-alias-select-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias s=select; \
         s item in alpha beta; do echo chosen:$item; break; done <<< '2' > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_select_accepts_brace_group_body() {
    let output_path = "target/rubash-alias-select-brace-body-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias s=select; \
         s item in alpha beta; {{ echo chosen:$item; break; }} <<< '2' > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_alias_introduced_select_without_in_uses_positional_params() {
    let output_path = "target/rubash-alias-select-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -s expand_aliases; alias s=select; \
         s item; do echo chosen:$item; break; done <<< '1' > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_positional_params(vec!["alpha".to_string()]);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "chosen:alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_pipeline_modifier_executes_full_pipeline() {
    let output_path = "target/rubash-time-pipeline-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time -p echo alpha | wc -l > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands[0].words[0], "time");
    assert!(ast.commands[0].pipe.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_pipeline_modifier_preserves_pipeline_status() {
    let output_path = "target/rubash-time-pipeline-status-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time echo alpha | grep beta > {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].pipe.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_command_redirects_timed_command_stdout() {
    let output_path = "target/rubash-time-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time -p echo hi > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands[0].words[0], "time");
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "hi\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_command_inverts_timed_status_with_redirect() {
    let output_path = "target/rubash-time-command-invert-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("time ! false > {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands[0].words, ["time", "!", "false"]);
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_for_command() {
    let output_path = "target/rubash-time-for-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p for x in a b; do echo $x >> {output_path}; done; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands[0].words, ["time", "-p"]);
    assert!(ast.commands[0].for_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\nb\nstatus:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_inversion_prefix_executes_for_command() {
    let output_path = "target/rubash-time-inverted-for-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p ! for x in a; do echo $x >> {output_path}; done; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands[0].words, ["time", "-p", "!"]);
    assert!(ast.commands[0].for_command.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a\nstatus:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_if_command_sequence() {
    let output_path = "target/rubash-time-if-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p if true; then echo yes > {output_path}; else echo no > {output_path}; fi; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands[0].words, ["time", "-p", "if", "true"]);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\nstatus:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_inversion_prefix_executes_if_command_sequence() {
    let output_path = "target/rubash-time-inverted-if-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p ! if true; then echo yes > {output_path}; else echo no > {output_path}; fi; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands[0].words, ["time", "-p", "!", "if", "true"]);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\nstatus:1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_while_command_sequence() {
    let output_path = "target/rubash-time-while-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=0; time -p while [[ $n -lt 2 ]]; do echo $n >> {output_path}; (( n++ )); done; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(
        ast.commands[1].words,
        ["time", "-p", "while", "[[", "$n", "-lt", "2", "]]"]
    );
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\nstatus:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_until_command_sequence() {
    let output_path = "target/rubash-time-until-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=0; time -p until [[ $n -ge 2 ]]; do echo $n >> {output_path}; (( n++ )); done; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(
        ast.commands[1].words,
        ["time", "-p", "until", "[[", "$n", "-ge", "2", "]]"]
    );
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\nstatus:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_brace_group() {
    let output_path = "target/rubash-time-brace-group-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "time -p {{ echo one > {output_path}; echo two >> {output_path}; }}; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands[0].words, ["time", "-p"]);
    assert!(ast.commands[0].brace_group.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "one\ntwo\nstatus:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_time_prefix_executes_subshell_group() {
    let output_path = "target/rubash-time-subshell-group-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=outer; time -p ( value=inner; echo $value > {output_path} ); echo $value >> {output_path}; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert_eq!(ast.commands[1].words, ["time", "-p"]);
    let body = ast.commands[1].brace_group.as_ref().unwrap();
    assert!(body[0].subshell);
    assert!(body[1].subshell_end);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "inner\nouter\nstatus:0\n"
    );
    let _ = fs::remove_file(output_path);
}
