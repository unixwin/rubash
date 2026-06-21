use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;
use std::fs;

#[test]
fn test_parameter_transform_quotes_value() {
    let output_path = "target/rubash-param-transform-quote-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v='two words'; echo ${{v@Q}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "'two words'\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_transform_changes_case() {
    let output_path = "target/rubash-param-transform-case-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=AbCd; echo ${{v@U}} ${{v@L}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ABCD abcd\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_transform_applies_to_positional_parameters() {
    let output_path = "target/rubash-param-transform-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("function p {{ echo ${{1@U}} ${{@@Q}} > {output_path}; }}; p alpha 'two words'");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "ALPHA alpha 'two words'\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_transform_quotes_array_elements() {
    let output_path = "target/rubash-param-transform-array-quote-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset arr; mapfile -t arr <<< $'two words\\nplain'; echo ${{arr[@]@Q}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "'two words' plain\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_transform_changes_array_element_case() {
    let output_path = "target/rubash-param-transform-array-case-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("arr=(AbC dEf); echo ${{arr[*]@U}} / ${{arr[@]@L}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "ABC DEF / abc def\n"
    );
    let _ = fs::remove_file(output_path);
}
