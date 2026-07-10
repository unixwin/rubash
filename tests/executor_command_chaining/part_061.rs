use super::super::*;
use std::fs;

#[test]
fn test_nounset_errors_for_unbound_assignment_prefix() {
    let output_path = "target/rubash-nounset-assignment-prefix-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset RUBASH_NOUNSET_PREFIX; set -u; value=$RUBASH_NOUNSET_PREFIX echo after > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(127))));
    assert_eq!(executor.last_exit_code(), 127);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_braced_positional_parameters_expand() {
    let output_path = "target/rubash-braced-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("function p {{ echo ${{1}} ${{2}} ${{#}} > {output_path}; }}; p alpha beta");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha beta 2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_positional_parameter_lengths_expand() {
    let output_path = "target/rubash-positional-length-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "function p {{ echo ${{#1}} ${{#2}} ${{#@}} ${{#*}} > {output_path}; }}; p alpha beta"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "5 4 2 2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_special_parameter_length_expands() {
    let output_path = "target/rubash-special-length-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("false; echo ${{#?}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_colon_plus_requires_non_empty_value() {
    let output_path = "target/rubash-param-colon-plus-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset v; echo unset:${{v:+alt}} > {output_path}; v=; echo empty:${{v:+alt}} >> {output_path}; v=x; echo set:${{v:+alt}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "unset:\nempty:\nset:alt\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_plus_expands_for_empty_set_value() {
    let output_path = "target/rubash-param-plus-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset v; echo unset:${{v+alt}} > {output_path}; v=; echo empty:${{v+alt}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "unset:\nempty:alt\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_default_ifs_is_set_for_parameter_plus_expansion() {
    let output_path = "target/rubash-default-ifs-plus-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "printf '%s\\n' \"foo ${{IFS+\"b   c\"}} baz\" > {output_path}; \
         set -o posix; printf '%s\\n' \"foo ${{IFS+'bar}} baz\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "foo b   c baz\nfoo 'bar baz\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_colon_question_errors_for_unset_value() {
    let output_path = "target/rubash-param-colon-question-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("unset v; echo ${{v:?boom}} > {output_path}; echo after > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
    assert_eq!(executor.last_exit_code(), 1);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_array_element_parameter_colon_question_errors_for_unset_value() {
    let output_path = "target/rubash-array-element-colon-question-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("arr=(ok); echo ${{arr[1]:?boom}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
    assert_eq!(executor.last_exit_code(), 1);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_parameter_question_allows_empty_set_value() {
    let output_path = "target/rubash-param-question-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=; echo ok:${{v?boom}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ok:\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_dash_uses_default_only_when_unset() {
    let output_path = "target/rubash-param-dash-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset v; echo unset:${{v-default}} > {output_path}; v=; echo empty:${{v-default}} >> {output_path}; v=x; echo set:${{v-default}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "unset:default\nempty:\nset:x\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_colon_dash_uses_default_for_empty_value() {
    let output_path = "target/rubash-param-colon-dash-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=; echo empty:${{v:-default}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "empty:default\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_element_parameter_operators_use_element_state() {
    let output_path = "target/rubash-array-element-param-operators-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(value); declare -A assoc; assoc[empty]=; assoc[key]=value; \
         printf '<%s>|<%s>|<%s>|<%s>\\n' \
         \"${{arr[0]:+alt}}\" \"${{arr[1]-missing}}\" \
         \"${{assoc[empty]:-fallback}}\" \"${{assoc[key]+alt}}\" \
         > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<alt>|<missing>|<fallback>|<alt>\n"
    );
    let _ = fs::remove_file(output_path);
}
