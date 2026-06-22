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
fn test_parameter_transform_expands_ansi_c_escapes() {
    let output_path = "target/rubash-param-transform-escape-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v='a\\nb'; printf '<%s>' \"${{v@E}}\" > {output_path}; v='\\141\\x42'; printf '<%s>' \"${{v@E}}\" >> {output_path}; v='a\\qb'; printf '<%s>\\n' \"${{v@E}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<a\nb><aB><a\\qb>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_transform_prints_assignment() {
    let output_path = "target/rubash-param-transform-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=\"two words\"; printf '<%s>\\n' \"${{v@A}}\" > {output_path}; v=\"a'b\"; printf '<%s>\\n' \"${{v@A}}\" >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<v='two words'>\n<v='a'\\''b'>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_transform_assignment_includes_attributes() {
    let output_path = "target/rubash-param-transform-assignment-attrs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("export x=value; readonly r=value; declare -i n=7; printf '<%s>\\n' \"${{x@A}}\" \"${{r@A}}\" \"${{n@A}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<declare -x x='value'>\n<declare -r r='value'>\n<declare -i n='7'>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_transform_assignment_prints_indexed_arrays() {
    let output_path = "target/rubash-param-transform-assignment-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(alpha beta); printf '<%s>\\n' \"${{arr@A}}\" \"${{arr[1]@A}}\" \"${{arr[*]@A}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<declare -a arr='alpha'>\n<declare -a arr='beta'>\n<declare -a arr=([0]=\"alpha\" [1]=\"beta\")>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_transform_assignment_prints_assoc_arrays() {
    let output_path = "target/rubash-param-transform-assignment-assoc-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("declare -A assoc; assoc[one]=alpha; assoc[two]=beta; printf '<%s>\\n' \"${{assoc@A}}\" \"${{assoc[one]@A}}\" \"${{assoc[*]@A}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<declare -A assoc>\n<declare -A assoc='alpha'>\n<declare -A assoc=([one]=\"alpha\" [two]=\"beta\" )>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_transform_prints_attributes() {
    let output_path = "target/rubash-param-transform-attrs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=value; export x=value; readonly r=value; declare -i n=7; arr=(alpha beta); declare -A assoc; assoc[one]=alpha; printf '<%s>\\n' \"${{v@a}}\" \"${{x@a}}\" \"${{r@a}}\" \"${{n@a}}\" \"${{arr@a}}\" \"${{arr[1]@a}}\" \"${{assoc@a}}\" \"${{assoc[one]@a}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<>\n<x>\n<r>\n<i>\n<a>\n<a>\n<A>\n<A>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_transform_prints_combined_attributes() {
    let output_path = "target/rubash-param-transform-combined-attrs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("declare -ix xi=7; readonly -a ra=(alpha); printf '<%s>\\n' \"${{xi@a}}\" \"${{ra@a}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<ix>\n<ar>\n");
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
