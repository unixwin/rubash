use super::super::*;
use std::fs;

#[test]
fn test_case_command_redirects_clause_stdout() {
    let output_path = "target/rubash-case-command-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "case x in x) echo matched ;; *) echo missed ;; esac > {output_path}; echo done >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].case_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "matched\ndone\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_command_redirect_creates_file_without_match() {
    let output_path = "target/rubash-case-command-redirect-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("case z in x) echo missed ;; esac > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    assert!(ast.commands[0].case_command.is_some());
    assert!(ast.commands[0].redirect_out.is_some());
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}
