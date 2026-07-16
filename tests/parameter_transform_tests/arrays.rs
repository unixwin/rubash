use super::*;

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
        "'two words' 'plain'\n"
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

#[test]
fn test_parameter_transform_expands_prompt_array_elements() {
    let output_path = "target/rubash-param-transform-array-prompt-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "USER=alice; HOSTNAME=box.example; arr=('\\u' '\\h'); \
         printf '<%s>\\n' \"${{arr[@]@P}}\" \"${{arr[0]@P}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<alice box>\n<alice>\n"
    );
    let _ = fs::remove_file(output_path);
}
