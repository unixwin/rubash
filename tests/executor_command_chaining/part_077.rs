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
