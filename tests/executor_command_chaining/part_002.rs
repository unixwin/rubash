use super::super::*;
use std::fs;

#[test]
fn test_pipefail_status_uses_rightmost_failing_command() {
    let output_path = "target/rubash-pipefail-status-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -o pipefail; false | true; echo $? > {output_path}; \
         true | false | true; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pipefail_status_keeps_pipestatus_entries() {
    let output_path = "target/rubash-pipefail-pipestatus-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("set -o pipefail; false | true; echo $? -- ${{PIPESTATUS[@]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1 -- 1 0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_pipefail_status_preserves_pipeline_output() {
    let output_path = "target/rubash-pipefail-output-status.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -o pipefail; printf 'a\\nb\\n' | grep z | wc -l > {output_path}; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_inverted_pipeline_uses_pipefail_status() {
    let output_path = "target/rubash-pipefail-invert-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -o pipefail; ! false | true; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_condition_bang_inverts_command_without_running_bang() {
    let output_path = "target/rubash-if-bang-condition-output.txt";
    let error_path = "target/rubash-if-bang-condition-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "function present() {{ :; }}; if ! type -t present >/dev/null 2>{error_path}; then echo missing > {output_path}; else echo found > {output_path}; fi"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "found\n");
    assert_eq!(fs::read_to_string(error_path).unwrap_or_default(), "");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_inline_then_executes_if_body_tail() {
    let output_path = "target/rubash-inline-then-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("if true; then echo yes > {output_path}; else echo no > {output_path}; fi");
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
fn test_nested_inline_if_scans_quoted_words_with_arithmetic_for() {
    let output_path = "target/rubash-nested-inline-if-for-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "command='String  \"4 0 0 0 0\"    validateAndParse 4'; words=($command); if [[ ${{words[1]}} =~ ^\\\" ]]; then if [[ ${{words[1]}} =~ \\\"$ ]]; then nextWord=2; else for ((nextWord=2;;nextWord++)); do if [[ ${{words[nextWord]}} =~ \\\"$ ]]; then ((nextWord++)); break; fi; done; fi; else nextWord=2; fi; echo next:$nextWord word:${{words[nextWord]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "next:6 word:validateAndParse\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_body_keeps_arithmetic_for_semicolons_inside_nested_if() {
    let output_path = "target/rubash-case-arithmetic-for-semicolons-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "command='String  \"4 0 0 0 0\"    validateAndParse 4'; words=($command); case ${{words[0]}} in String) if [[ ${{words[1]}} =~ ^\\\" ]]; then if [[ ${{words[1]}} =~ \\\"$ ]]; then nextWord=2; else for ((nextWord=2;;nextWord++)); do if [[ ${{words[nextWord]}} =~ \\\"$ ]]; then ((nextWord++)); break; fi; done; fi; else nextWord=2; fi; echo next:$nextWord word:${{words[nextWord]}} > {output_path} ;; esac"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "next:6 word:validateAndParse\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unquoted_parameter_array_assignment_preserves_quote_characters() {
    let output_path = "target/rubash-array-assignment-quoted-fields-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "command='String  \"4 0 0 0 0\"    validateAndParse 4'; words=($command); full=\"${{words[*]}}\"; echo \"$full\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "String \"4 0 0 0 0\" validateAndParse 4\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_compound_array_assignment_preserves_quoted_fields() {
    let output_path = "target/rubash-compound-array-quoted-fields-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("foo=(a 'b c' d); printf '<%s>\\n' \"${{foo[@]}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<a>\n<b c>\n<d>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_compound_array_assignment_preserves_quoted_array_at() {
    let output_path = "target/rubash-compound-array-quoted-at-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "foo=(a 'b c'); foo=(\"${{foo[@]}}\" d); printf '<%s>\\n' \"${{foo[@]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<a>\n<b c>\n<d>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_compound_array_assignment_preserves_quoted_array_slice() {
    let output_path = "target/rubash-compound-array-quoted-slice-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "foo=(a 'b c' d e); foo=(\"${{foo[@]:0:${{#foo[@]}}-1}}\"); printf '<%s>\\n' \"${{foo[@]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<a>\n<b c>\n<d>\n"
    );
    let _ = fs::remove_file(output_path);
}
