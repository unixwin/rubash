use super::super::*;
use std::fs;

fn run_alias_if_script(input: String, output_path: &str) {
    let _ = fs::remove_file(output_path);
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
fn test_alias_introduced_elif_executes_branch() {
    let output_path = "target/rubash-alias-elif-output.txt";
    run_alias_if_script(
        format!(
            "shopt -s expand_aliases\nalias e=elif\nif false; then echo no > {output_path}; e true; then echo yes > {output_path}; else echo bad > {output_path}; fi"
        ),
        output_path,
    );
}

#[test]
fn test_alias_introduced_else_executes_branch() {
    let output_path = "target/rubash-alias-else-output.txt";
    run_alias_if_script(
        format!(
            "shopt -s expand_aliases\nalias el=else\nif false; then echo bad > {output_path}; el echo yes > {output_path}; fi"
        ),
        output_path,
    );
}

#[test]
fn test_alias_introduced_then_starts_body() {
    let output_path = "target/rubash-alias-then-output.txt";
    run_alias_if_script(
        format!("shopt -s expand_aliases\nalias t=then\nif true; t echo yes > {output_path}; fi"),
        output_path,
    );
}

#[test]
fn test_alias_introduced_fi_closes_if() {
    let output_path = "target/rubash-alias-fi-output.txt";
    run_alias_if_script(
        format!("shopt -s expand_aliases\nalias f=fi\nif true; then echo yes > {output_path}; f"),
        output_path,
    );
}

#[test]
fn test_alias_introduced_nested_if_does_not_close_outer_if() {
    let output_path = "target/rubash-alias-nested-if-output.txt";
    run_alias_if_script(
        format!(
            "shopt -s expand_aliases\nalias i=if\nif true; then i true; then echo yes > {output_path}; fi; else echo no > {output_path}; fi"
        ),
        output_path,
    );
}
