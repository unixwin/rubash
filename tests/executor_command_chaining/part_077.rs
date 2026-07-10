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
