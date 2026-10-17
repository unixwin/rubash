use super::super::*;
use std::fs;

#[test]
fn test_unquoted_positional_at_splits_each_parameter_and_empty_ifs_joins_star_without_separator() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-positional-ifs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -- a 'b c'; printf '<%s>\\n' $@ > {output_path}; \
         IFS=; set -- a b; printf '<%s>\\n' \"$*\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<a>\n<b>\n<c>\n<ab>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_assignment_operator_expands_in_command_word_order() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-parameter-operator-order-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "x=; printf '<%s><%s><%s>\\n' \"${{x:-a}}\" \"${{x:=b}}\" \"${{x:+c}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<a><b><c>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_parameter_pattern_removal_applies_to_each_value() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-array-pattern-removal-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("arr=(src/main.rs src/lib.rs); echo ${{arr[@]#*/}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "main.rs lib.rs\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_array_star_pattern_ops_use_ifs_first_char() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-array-star-pattern-ifs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "IFS=,; arr=(src/main.rs src/lib.rs); \
         printf 'pat<%s>\\n' \"${{arr[*]#*/}}\" > {output_path}; \
         arr=(banana gamma); \
         printf 'rep<%s>\\n' \"${{arr[*]//a/o}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "pat<main.rs,lib.rs>\nrep<bonono,gommo>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_element_parameter_substring_uses_element_value() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-array-element-substring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero alpha); declare -A assoc; assoc[key]=gamma; \
         echo ${{arr[1]:1:3}} ${{assoc[key]:2}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "lph mma\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_positional_parameter_substring_uses_offset_and_length() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-positional-substring-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "function p {{ echo ${{@:2:2}} / ${{*:3}} > {output_path}; }}; p alpha beta gamma delta"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "beta gamma / gamma delta\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unquoted_positional_parameter_substring_splits_words() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-positional-substring-split-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "function p {{ for x in ${{@:2}}; do echo \"$x\" >> {output_path}; done; }}; p arr 0 2 3"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n2\n3\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_positional_substring_preserves_quoted_and_splits_unquoted_words() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-positional-substring-word-boundaries-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -- zero 'one two' three; \
         printf 'quoted<%s>\\n' \"${{@:2:2}}\" > {output_path}; \
         printf 'unquoted<%s>\\n' ${{@:2:2}} >> {output_path}; \
         printf 'star<%s>\\n' \"${{*:2:2}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "quoted<one two>\nquoted<three>\nunquoted<one>\nunquoted<two>\nunquoted<three>\nstar<one two three>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_positional_at_expands_to_separate_arguments() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-quoted-positional-at-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -- 3 1 2; printf '[%s]\\n' \"${{@}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "[3]\n[1]\n[2]\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_set_positional_operands_split_unquoted_variable_with_custom_ifs() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-set-positional-custom-ifs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        r#"line='a"b"c'; old=$IFS; IFS='"'; set -- $line; IFS=$old; printf 'n=%s one=%s two=%s three=%s\n' "$#" "$1" "$2" "$3" > {output_path}"#
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "n=3 one=a two=b three=c\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_positional_parameter_substring_supports_negative_offset() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-positional-substring-negative-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function p {{ echo ${{@: -2:1}} > {output_path}; }}; p alpha beta gamma");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_positional_parameter_pattern_removal_applies_to_each_value() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-positional-pattern-removal-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -- alpha.tmp beta.tmp; echo ${{@%.tmp}} / ${{1%%.*}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alpha beta / alpha\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_element_parameter_operations_use_element_value() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-array-element-param-ops-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(src/main.rs alpha beta); declare -A assoc; assoc[key]=src/lib.rs; \
         echo ${{arr[0]#*/}} ${{arr[1]/a/o}} ${{arr[2]^}} ${{assoc[key]#*/}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "main.rs olpha Beta lib.rs\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_replacement_replaces_first_and_all_matches() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-param-replace-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=banana; echo ${{v/a/o}} ${{v//a/o}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "bonana bonono\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_replacement_deletes_matches() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-param-replace-delete-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=banana; echo ${{v/a}} ${{v//a}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "bnana bnn\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_replacement_uses_shell_patterns() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-param-replace-pattern-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=abcd; echo ${{v/?b/X}} ${{v//?/x}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "Xcd xxxx\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_replacement_supports_negated_bracket_patterns() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-param-replace-negated-class-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v='a-12b'; echo ${{v//[^0-9]}} ${{v//[^-]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "12 -\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_replacement_supports_prefix_and_suffix_anchors() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-param-replace-anchor-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("v=abcabc; echo ${{v/#abc/X}} ${{v/%abc/X}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "Xabc abcX\n");
    let _ = fs::remove_file(output_path);
}
