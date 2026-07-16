use super::super::*;
use std::fs;

#[test]
fn test_parameter_colon_equals_assigns_empty_value() {
    let output_path = "target/rubash-param-colon-equals-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=; : ${{v:=default}}; echo $v > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "default\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_assignment_parameter_operators_preserve_word_with_dash() {
    let output_path = "target/rubash-param-quoted-assignment-operator-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset v w; \
         printf '<%s:%s>\\n' \"${{v:=a-b}}\" \"$v\" > {output_path}; \
         printf '<%s:%s>\\n' \"${{w=a-b}}\" \"$w\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<a-b:a-b>\n<a-b:a-b>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_question_operator_distinguishes_null_from_unset() {
    let output_path = "target/rubash-param-question-operator-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=; printf '<%s>\\n' \"${{v?empty-ok}}\" > {output_path}; \
         v=value; printf '<%s>\\n' \"${{v:?nonempty}}\" >> {output_path}; \
         unset v; echo ${{v?missing}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_err());
    assert_eq!(executor.last_exit_code(), 1);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<>\n<value>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_pattern_removes_prefixes_and_suffixes() {
    let output_path = "target/rubash-param-pattern-remove-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("v=abc_def_ghi; echo ${{v#*_}} ${{v##*_}} ${{v%_*}} ${{v%%_*}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "def_ghi ghi abc_def abc\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_pattern_removal_decodes_quoted_pattern_words() {
    let output_path = "target/rubash-param-pattern-quoted-word-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "pat=$'no\\t'; x=$'no\\tOK'; y=notOK; \
         echo 1:${{x#$'no\\t'}} 2:O${{x#$'no\\t'O}} 3:${{x#n$'o\\t'}} 4:${{x#'no\t'}} 5:${{x#$pat}} 6:${{y#$'not'}} 7:${{y#'not'}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "1:OK 2:OK 3:OK 4:OK 5:OK 6:OK 7:OK\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_replacement_decodes_quoted_pattern_words() {
    let output_path = "target/rubash-param-replacement-quoted-word-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=\"'\"; printf '<%s>\\n' \
         \"${{v/$'\\''/x}}\" \
         ${{v/$'\\''/x}} \
         \"${{v/\\'/x}}\" \
         ${{v/\\'/x}} \
         \"${{v/x/\\'}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<x>\n<x>\n<x>\n<x>\n<'>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_replacement_preserves_double_quoted_single_quote_pattern() {
    let output_path = "target/rubash-param-replacement-double-quoted-single-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "test=\"weferfds'dsfsdf\"; \
         printf '<%s>\\n' \"'${{test//\"'\"/}}'\" \
         \"${{test//\"'\"/\"'\\\\''\"}}\" \
         \"'${{test//\"'\"/\"'\\\\''\"}}'\" \
         \\'${{test//\"'\"/\\'\\\\\\'\\'}}\\'\" \" \
         \"'\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<'weferfdsdsfsdf'>\n<weferfds'\\''dsfsdf>\n<'weferfds'\\''dsfsdf'>\n<'weferfds'\\''dsfsdf' >\n<'>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_substring_uses_offset_and_length() {
    let output_path = "target/rubash-param-substring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=abcdef; echo ${{v:2:3}} ${{v:3}} ${{v::3}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "cde def abc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_substring_slices_characters() {
    let output_path = "target/rubash-param-substring-utf8-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=aßcd; echo ${{v:1:2}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ßc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_substring_supports_negative_offset() {
    let output_path = "target/rubash-param-substring-negative-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=abcdef; echo ${{v: -2}} ${{v: -4:2}} \"${{v: -2}}\" \"${{v: -4:2}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ef cd ef cd\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_substring_accepts_arithmetic_offset() {
    let output_path = "target/rubash-param-substring-arith-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=abcdef; n=3; echo ${{v:(-$n)}} ${{v:0:(-$n)}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "def abc\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_substring_does_not_shadow_colon_dash_default() {
    let output_path = "target/rubash-param-substring-default-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("unset v; echo ${{v:-fallback}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "fallback\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_parameter_substring_uses_offset_and_length() {
    let output_path = "target/rubash-array-substring-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("arr=(zero one two three); echo ${{arr[@]:1:2}} / ${{arr[@]::2}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "one two / zero one\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_parameter_substring_supports_negative_offset() {
    let output_path = "target/rubash-array-substring-negative-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one two three); echo ${{arr[*]: -2}} / \"${{arr[*]: -2}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "two three / two three\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_sparse_array_parameter_substring_slices_values() {
    let output_path = "target/rubash-sparse-array-substring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta\\ngamma'; echo ${{arr[@]:1:1}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "beta\n");
    let _ = fs::remove_file(output_path);
}
