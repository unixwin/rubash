use super::super::*;
use std::fs;

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
    let input = format!("function p {{ echo ${{@/%a/Z}} > {output_path}; }}; p alpha beta gamma");
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
    let input = format!("function p {{ echo ${{@^^}} / ${{1,,}} > {output_path}; }}; p alpha BETA");
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
