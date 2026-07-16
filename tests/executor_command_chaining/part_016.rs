use super::super::*;
use std::fs;

#[test]
fn test_unset_assoc_array_element() {
    let output_path = "target/rubash-unset-assoc-element-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("declare -A assoc; assoc[one]=alpha; assoc[two]=beta; unset 'assoc[one]'; echo ${{!assoc[@]}} > {output_path}; echo ${{assoc[one]}} >> {output_path}; echo ${{assoc[two]}} >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "two\n\nbeta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_assoc_compound_assignment_accepts_key_value_pairs() {
    let output_path = "target/rubash-declare-assoc-pairs-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_PAIRS");
    let input = format!(
        "declare -A RUBASH_ASSOC_PAIRS=(one alpha two beta three); \
         printf '%s/%s/<%s>\\n' \"${{RUBASH_ASSOC_PAIRS[one]}}\" \"${{RUBASH_ASSOC_PAIRS[two]}}\" \"${{RUBASH_ASSOC_PAIRS[three]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha/beta/<>\n");
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_PAIRS");
}

#[test]
fn test_declare_assoc_compound_assignment_preserves_quoted_words() {
    let output_path = "target/rubash-declare-assoc-quotes-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_QUOTED_PAIRS");
    let input = format!(
        "declare -A RUBASH_ASSOC_QUOTED_PAIRS=(one \"two words\" three \"four words\"); \
         printf '<%s>/<%s>\\n' \"${{RUBASH_ASSOC_QUOTED_PAIRS[one]}}\" \"${{RUBASH_ASSOC_QUOTED_PAIRS[three]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<two words>/<four words>\n"
    );
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_QUOTED_PAIRS");
}

#[test]
fn test_declare_assoc_compound_assignment_preserves_quoted_keys() {
    let output_path = "target/rubash-declare-assoc-quoted-keys-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_QUOTED_KEYS");
    let input = format!(
        "declare -A RUBASH_ASSOC_QUOTED_KEYS=([\"two words\"]=\"value here\"); \
         declare -p RUBASH_ASSOC_QUOTED_KEYS > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "declare -A RUBASH_ASSOC_QUOTED_KEYS=([\"two words\"]=\"value here\" )\n"
    );
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_QUOTED_KEYS");
}

#[test]
fn test_declare_assoc_compound_assignment_accepts_unquoted_keys_with_spaces() {
    let output_path = "target/rubash-declare-assoc-spaced-keys-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_SPACED_KEYS");
    let input = format!(
        "declare -A RUBASH_ASSOC_SPACED_KEYS=([two words]=v [other key]=w); \
         printf '%s/%s\\n' \"${{RUBASH_ASSOC_SPACED_KEYS[two words]}}\" \
         \"${{RUBASH_ASSOC_SPACED_KEYS[other key]}}\" > {output_path}; \
         declare -p RUBASH_ASSOC_SPACED_KEYS >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "v/w\ndeclare -A RUBASH_ASSOC_SPACED_KEYS=([\"two words\"]=\"v\" [\"other key\"]=\"w\" )\n"
    );
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_SPACED_KEYS");
}

#[test]
fn test_assoc_parameter_expansion_accepts_quoted_keys() {
    let output_path = "target/rubash-assoc-quoted-key-expansion-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_QUOTED_LOOKUP");
    let input = format!(
        "declare -A RUBASH_ASSOC_QUOTED_LOOKUP=([\"two words\"]=\"value here\"); \
         key=\"two words\"; printf '<%s>\\n' \
         \"${{RUBASH_ASSOC_QUOTED_LOOKUP[$key]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<value here>\n");
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_QUOTED_LOOKUP");
}

#[test]
fn test_assoc_parameter_expansion_accepts_direct_quoted_keys() {
    let output_path = "target/rubash-assoc-direct-quoted-key-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_DIRECT_QUOTED_LOOKUP");
    let input = format!(
        "declare -A RUBASH_ASSOC_DIRECT_QUOTED_LOOKUP=([\"two words\"]=\"value here\"); \
         printf '<%s>\\n' \"${{RUBASH_ASSOC_DIRECT_QUOTED_LOOKUP[\"two words\"]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<value here>\n");
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_DIRECT_QUOTED_LOOKUP");
}

#[test]
fn test_assoc_element_assignment_accepts_expanded_quoted_keys() {
    let output_path = "target/rubash-assoc-expanded-key-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_EXPANDED_ASSIGN");
    let input = format!(
        "declare -A RUBASH_ASSOC_EXPANDED_ASSIGN=([\"two words\"]=\"value here\"); \
         key=\"two words\"; RUBASH_ASSOC_EXPANDED_ASSIGN[$key]+=!; \
         printf '<%s>\\n' \"${{RUBASH_ASSOC_EXPANDED_ASSIGN[$key]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<value here!>\n");
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_EXPANDED_ASSIGN");
}

#[test]
fn test_variable_is_set_accepts_quoted_associative_keys() {
    let output_path = "target/rubash-assoc-quoted-key-test-v-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_TEST_V");
    let input = format!(
        "declare -A RUBASH_ASSOC_TEST_V=([\"two words\"]=\"value here\"); \
         test -v 'RUBASH_ASSOC_TEST_V[two words]'; echo test:$? > {output_path}; \
         [[ -v RUBASH_ASSOC_TEST_V[\"two words\"] ]]; echo cond:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "test:0\ncond:0\n");
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_TEST_V");
}

#[test]
fn test_assoc_compound_append_accepts_key_value_pairs() {
    let output_path = "target/rubash-assoc-append-pairs-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_APPEND_PAIRS");
    let input = format!(
        "declare -A RUBASH_ASSOC_APPEND_PAIRS=(one alpha); RUBASH_ASSOC_APPEND_PAIRS+=(two beta three); \
         printf '%s/%s/<%s>\\n' \"${{RUBASH_ASSOC_APPEND_PAIRS[one]}}\" \"${{RUBASH_ASSOC_APPEND_PAIRS[two]}}\" \"${{RUBASH_ASSOC_APPEND_PAIRS[three]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha/beta/<>\n");
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_APPEND_PAIRS");
}

#[test]
fn test_assoc_compound_append_accepts_element_append_assignments() {
    let output_path = "target/rubash-assoc-append-element-assign-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_APPEND_ELEMENTS");
    let input = format!(
        "declare -A RUBASH_ASSOC_APPEND_ELEMENTS=([a]=1); \
         RUBASH_ASSOC_APPEND_ELEMENTS+=([a]+=2 [b]+=x); \
         printf '%s:%s\\n' \"${{RUBASH_ASSOC_APPEND_ELEMENTS[a]}}\" \
         \"${{RUBASH_ASSOC_APPEND_ELEMENTS[b]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "12:x\n");
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_APPEND_ELEMENTS");
}

#[test]
fn test_declare_assoc_append_accepts_element_append_assignments() {
    let output_path = "target/rubash-declare-assoc-append-element-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_DECLARE_ASSOC_APPEND_ELEMENTS");
    let input = format!(
        "declare -A RUBASH_DECLARE_ASSOC_APPEND_ELEMENTS=([a]=1); \
         declare -A RUBASH_DECLARE_ASSOC_APPEND_ELEMENTS+=([a]+=2 [b]+=x); \
         printf '%s:%s\\n' \"${{RUBASH_DECLARE_ASSOC_APPEND_ELEMENTS[a]}}\" \
         \"${{RUBASH_DECLARE_ASSOC_APPEND_ELEMENTS[b]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "12:x\n");
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_DECLARE_ASSOC_APPEND_ELEMENTS");
}

#[test]
fn test_assoc_scalar_append_assigns_zero_key() {
    let output_path = "target/rubash-assoc-scalar-append-output.txt";
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_SCALAR_APPEND");
    let input = format!(
        "declare -A RUBASH_ASSOC_SCALAR_APPEND=([one]=bar); \
         RUBASH_ASSOC_SCALAR_APPEND+=zero; \
         RUBASH_ASSOC_SCALAR_APPEND+=([four]=four); \
         declare -p RUBASH_ASSOC_SCALAR_APPEND > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "declare -A RUBASH_ASSOC_SCALAR_APPEND=([four]=\"four\" [0]=\"zero\" [one]=\"bar\" )\n"
    );
    let _ = fs::remove_file(output_path);
    std::env::remove_var("RUBASH_ASSOC_SCALAR_APPEND");
}

#[test]
fn test_declare_p_redirects_output() {
    let output_path = "target/rubash-declare-p-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=value; declare -p v > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "declare -- v=\"value\"\n"
    );
    let _ = fs::remove_file(output_path);
}
