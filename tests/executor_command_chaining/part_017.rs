use super::super::*;
use std::fs;

#[test]
fn test_declare_p_appends_output() {
    let output_path = "target/rubash-declare-p-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("v=value; declare -p v >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\ndeclare -- v=\"value\"\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_p_without_names_lists_variables() {
    let output_path = "target/rubash-declare-p-all-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("RUBASH_DECLARE_ALL=value; declare -p > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("declare -- RUBASH_DECLARE_ALL=\"value\"\n"));
    assert!(!output.contains("__RUBASH_EXPORTED_VARS"));
    std::env::remove_var("RUBASH_DECLARE_ALL");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_without_names_lists_variables() {
    let output_path = "target/rubash-declare-all-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("RUBASH_DECLARE_BARE=value; declare > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert!(fs::read_to_string(output_path)
        .unwrap()
        .contains("declare -- RUBASH_DECLARE_BARE=\"value\"\n"));
    std::env::remove_var("RUBASH_DECLARE_BARE");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_attribute_filters_without_names() {
    let output_path = "target/rubash-declare-attr-filter-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "plain=value; declare -i RUBASH_DECLARE_INT=7; \
         declare -u RUBASH_DECLARE_UP=abc; readonly RUBASH_DECLARE_RO=1; \
         declare -i > {output_path}; declare -u >> {output_path}; declare -r >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("declare -i RUBASH_DECLARE_INT=\"7\"\n"));
    assert!(output.contains("declare -u RUBASH_DECLARE_UP=\"ABC\"\n"));
    assert!(output.contains("declare -r RUBASH_DECLARE_RO=\"1\"\n"));
    assert!(!output.contains("plain=\"value\""));
    std::env::remove_var("RUBASH_DECLARE_INT");
    std::env::remove_var("RUBASH_DECLARE_UP");
    std::env::remove_var("RUBASH_DECLARE_RO");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_marks_inherited_environment_as_exported() {
    let output_path = "target/rubash-declare-inherited-export-output.txt";
    let _ = fs::remove_file(output_path);
    let old_value = std::env::var("RUBASH_INHERITED_EXPORT").ok();
    std::env::set_var("RUBASH_INHERITED_EXPORT", "from-env");
    let input = format!(
        "RUBASH_NOT_EXPORTED=local; \
         declare -p RUBASH_INHERITED_EXPORT > {output_path}; \
         declare -x >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    match old_value {
        Some(value) => std::env::set_var("RUBASH_INHERITED_EXPORT", value),
        None => std::env::remove_var("RUBASH_INHERITED_EXPORT"),
    }
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("declare -x RUBASH_INHERITED_EXPORT=\"from-env\"\n"));
    assert!(!output.contains("RUBASH_NOT_EXPORTED"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_r_marks_readonly_variable() {
    let output_path = "target/rubash-declare-readonly-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("declare -r RO=1; declare -p RO > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "declare -r RO=\"1\"\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_rx_marks_exported_readonly_variable() {
    let output_path = "target/rubash-declare-readonly-export-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("declare -rx REX=2; declare -p REX > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "declare -rx REX=\"2\"\n"
    );
    std::env::remove_var("REX");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_prefix_assignment_persists_as_exported_variable() {
    let output_path = target_test_path("rubash-declare-prefix-assignment-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "unset RUBASH_PREFIX_DECL RUBASH_PREFIX_OVERRIDE; \
         RUBASH_PREFIX_DECL=foo declare -r RUBASH_PREFIX_DECL; \
         RUBASH_PREFIX_OVERRIDE=bar declare -r RUBASH_PREFIX_OVERRIDE=qux; \
         echo value:$RUBASH_PREFIX_DECL > {shell_output_path}; \
         echo ${{RUBASH_PREFIX_DECL@A}} >> {shell_output_path}; \
         echo ${{RUBASH_PREFIX_OVERRIDE@A}} >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "value:foo\ndeclare -rx RUBASH_PREFIX_DECL='foo'\ndeclare -rx RUBASH_PREFIX_OVERRIDE='qux'\n"
    );
    std::env::remove_var("RUBASH_PREFIX_DECL");
    std::env::remove_var("RUBASH_PREFIX_OVERRIDE");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_p_preserves_combined_attribute_order() {
    let output_path = target_test_path("rubash-declare-combined-attrs-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "declare -lrx RUBASH_DECLARE_LRX=ABC; declare -p RUBASH_DECLARE_LRX > {shell_output_path}; \
         declare -ux RUBASH_DECLARE_UX=abc; declare -p RUBASH_DECLARE_UX >> {shell_output_path}; \
         declare -irx RUBASH_DECLARE_IRX=7; declare -p RUBASH_DECLARE_IRX >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "declare -rxl RUBASH_DECLARE_LRX=\"abc\"\n\
declare -xu RUBASH_DECLARE_UX=\"ABC\"\n\
declare -irx RUBASH_DECLARE_IRX=\"7\"\n"
    );
    std::env::remove_var("RUBASH_DECLARE_LRX");
    std::env::remove_var("RUBASH_DECLARE_UX");
    std::env::remove_var("RUBASH_DECLARE_IRX");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_declare_rx_without_assignment_marks_unset_variable() {
    let output_path = target_test_path("rubash-declare-rx-unset-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let input = format!(
        "unset RUBASH_DECLARE_RX_UNSET; declare -rx RUBASH_DECLARE_RX_UNSET; \
         printf '<%s>\\n' \"${{RUBASH_DECLARE_RX_UNSET-unset}}\" > {shell_output_path}; \
         declare -p RUBASH_DECLARE_RX_UNSET >> {shell_output_path}; \
         export -p >> {shell_output_path}; \
         readonly -p >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.starts_with("<unset>\ndeclare -rx RUBASH_DECLARE_RX_UNSET\n"));
    assert_eq!(
        output
            .matches("declare -rx RUBASH_DECLARE_RX_UNSET\n")
            .count(),
        3
    );
    assert!(!output.contains("RUBASH_DECLARE_RX_UNSET=\"\""));
    std::env::remove_var("RUBASH_DECLARE_RX_UNSET");
    let _ = fs::remove_file(&output_path);
}
