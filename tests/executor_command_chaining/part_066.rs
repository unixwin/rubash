use super::super::*;
use std::fs;

#[test]
fn test_indirect_parameter_uses_positional_parameter_name() {
    let output_path = "target/rubash-param-indirect-positional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function p {{ target=value; echo ${{!1}} > {output_path}; }}; p target");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "value\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indirect_prefix_expands_matching_variable_names() {
    let output_path = "target/rubash-param-indirect-prefix-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "RUBASH_INDIRECT_A=1; RUBASH_INDIRECT_B=2; echo ${{!RUBASH_INDIRECT_*}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "RUBASH_INDIRECT_A RUBASH_INDIRECT_B\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_indirect_array_parameter_transform_expands_first_value() {
    let output_path = "target/rubash-param-indirect-array-transform-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("arr=(alpha beta); ref=arr; echo ${{!ref[@]@Q}} ${{!ref[*]@U}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "'alpha' ALPHA\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_assignment_transform_matches_at_and_star_forms() {
    let output_path = "target/rubash-param-array-assignment-transform-output.txt";
    let _ = fs::remove_file(output_path);
    for name in [
        "RUBASH_ASSIGN_TRANSFORM_ARRAY",
        "RUBASH_ASSIGN_TRANSFORM_ASSOC",
    ] {
        std::env::remove_var(name);
    }
    let input = format!(
        "RUBASH_ASSIGN_TRANSFORM_ARRAY=(zero one); declare -A RUBASH_ASSIGN_TRANSFORM_ASSOC=([0]=z); \
         echo arr_star:${{RUBASH_ASSIGN_TRANSFORM_ARRAY[*]@A}} > {output_path}; \
         echo arr_at:${{RUBASH_ASSIGN_TRANSFORM_ARRAY[@]@A}} >> {output_path}; \
         echo assoc_scalar:${{RUBASH_ASSIGN_TRANSFORM_ASSOC@A}} >> {output_path}; \
         echo assoc_at:${{RUBASH_ASSIGN_TRANSFORM_ASSOC[@]@A}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "arr_star:declare -a RUBASH_ASSIGN_TRANSFORM_ARRAY=([0]=\"zero\" [1]=\"one\")\narr_at:declare -a RUBASH_ASSIGN_TRANSFORM_ARRAY=([0]=\"zero\" [1]=\"one\")\nassoc_scalar:declare -A RUBASH_ASSIGN_TRANSFORM_ASSOC='z'\nassoc_at:declare -A RUBASH_ASSIGN_TRANSFORM_ASSOC=([0]=\"z\" )\n"
    );
    let _ = fs::remove_file(output_path);
    for name in [
        "RUBASH_ASSIGN_TRANSFORM_ARRAY",
        "RUBASH_ASSIGN_TRANSFORM_ASSOC",
    ] {
        std::env::remove_var(name);
    }
}

#[test]
fn test_array_value_parameter_transforms_apply_to_elements() {
    let output_path = "target/rubash-param-array-value-transform-output.txt";
    let _ = fs::remove_file(output_path);
    for name in [
        "RUBASH_VALUE_TRANSFORM_ARRAY",
        "RUBASH_VALUE_TRANSFORM_ASSOC",
        "RUBASH_VALUE_TRANSFORM_REF",
    ] {
        std::env::remove_var(name);
    }
    let input = format!(
        "RUBASH_VALUE_TRANSFORM_ARRAY=('a b' c); \
         declare -A RUBASH_VALUE_TRANSFORM_ASSOC=([k]='v w' [0]=z); \
         declare -n RUBASH_VALUE_TRANSFORM_REF=RUBASH_VALUE_TRANSFORM_ARRAY; \
         echo arr_q:${{RUBASH_VALUE_TRANSFORM_ARRAY@Q}} > {output_path}; \
         echo arr0_q:${{RUBASH_VALUE_TRANSFORM_ARRAY[0]@Q}} >> {output_path}; \
         echo arr0_u:${{RUBASH_VALUE_TRANSFORM_ARRAY[0]@U}} >> {output_path}; \
         echo assoc_q:${{RUBASH_VALUE_TRANSFORM_ASSOC@Q}} >> {output_path}; \
         echo assoc_k_q:${{RUBASH_VALUE_TRANSFORM_ASSOC[k]@Q}} >> {output_path}; \
         echo ref_q:${{RUBASH_VALUE_TRANSFORM_REF@Q}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "arr_q:'a b'\narr0_q:'a b'\narr0_u:A B\nassoc_q:'z'\nassoc_k_q:'v w'\nref_q:'a b'\n"
    );
    let _ = fs::remove_file(output_path);
    for name in [
        "RUBASH_VALUE_TRANSFORM_ARRAY",
        "RUBASH_VALUE_TRANSFORM_ASSOC",
        "RUBASH_VALUE_TRANSFORM_REF",
    ] {
        std::env::remove_var(name);
    }
}

#[test]
fn test_indirect_array_pattern_removes_prefixes_and_suffixes() {
    let output_path = "target/rubash-param-indirect-array-pattern-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(aaa bbb); ref='arr[@]'; echo ${{!ref##aa}} > {output_path}; echo ${{!ref[@]%b}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a bbb\naaa bb\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_test_v_checks_array_subscripts() {
    let output_path = "target/rubash-test-v-array-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=value; arr=(zero one); declare -A assoc; assoc[one]=alpha; test -v 'v[0]'; echo $? > {output_path}; test -v 'v[1]'; echo $? >> {output_path}; test -v 'arr[1]'; echo $? >> {output_path}; test -v 'arr[9]'; echo $? >> {output_path}; test -v 'assoc[one]'; echo $? >> {output_path}; test -v 'assoc[two]'; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n1\n0\n1\n0\n1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_v_checks_array_subscripts() {
    let output_path = "target/rubash-conditional-v-array-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "v=value; arr=(zero one); declare -A assoc; assoc[one]=alpha; [[ -v v[0] ]]; echo $? > {output_path}; [[ -v v[1] ]]; echo $? >> {output_path}; [[ -v arr[1] ]]; echo $? >> {output_path}; [[ -v arr[9] ]]; echo $? >> {output_path}; [[ -v assoc[one] ]]; echo $? >> {output_path}; [[ -v assoc[two] ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n1\n0\n1\n0\n1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_r_checks_readonly_variables() {
    let output_path = "target/rubash-conditional-readonly-unary-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "readonly RUBASH_CONDITIONAL_READONLY_VAR=value; plain=value; \
         [[ -R RUBASH_CONDITIONAL_READONLY_VAR ]]; echo $? > {output_path}; \
         [[ -R plain ]]; echo $? >> {output_path}; \
         [[ -R UID ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_string_unary_checks_expanded_value() {
    let output_path = "target/rubash-conditional-string-unary-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "value=abc; empty=; [[ -n abc ]]; echo $? > {output_path}; [[ -n $empty ]]; echo $? >> {output_path}; [[ -z abc ]]; echo $? >> {output_path}; [[ -z $empty ]]; echo $? >> {output_path}; [[ -n $value ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n1\n0\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_binary_checks_expand_values() {
    let output_path = "target/rubash-conditional-binary-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "left=abc; right=def; n=3; [[ $left = abc ]]; echo $? > {output_path}; [[ $left != $right ]]; echo $? >> {output_path}; [[ $n -ne 4 ]]; echo $? >> {output_path}; [[ $n -lt 4 ]]; echo $? >> {output_path}; [[ $n -le 3 ]]; echo $? >> {output_path}; [[ $n -ge 3 ]]; echo $? >> {output_path}; [[ $n -gt 4 ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n0\n0\n0\n0\n0\n1\n"
    );
    let _ = fs::remove_file(output_path);
}
