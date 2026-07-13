use super::super::*;
use std::fs;

#[test]
fn test_export_f_marks_function_for_declare_export_listing() {
    let output_path = "target/rubash-export-f-output.txt";
    let status_path = "target/rubash-export-f-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "foo() {{ echo hi; }}; export -f foo; echo export:$? > {status_path}; \
         declare -xF > {output_path}; declare -xf >> {output_path}; export -pf >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "export:0\n");
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "declare -fx foo\ndeclare -fx foo\nfoo () \n{ \n    echo hi\n}\ndeclare -fx foo\nfoo () \n{ \n    echo hi\n}\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_hyphenated_function_name_can_be_called_and_exported() {
    let output_path = target_test_path("rubash-hyphen-function-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let input = format!(
        "foo-a() {{ echo local; }}; foo-a > {shell_output_path}; \
         export -f foo-a; {rubash} -c foo-a >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "local\nlocal\n");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_export_f_rejects_nonexportable_function_names() {
    let output_path = "target/rubash-export-nonexportable-function-output.txt";
    let error_path = "target/rubash-export-nonexportable-function-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "function foo=bar {{ echo equals; }}; foo\\=bar > {output_path}; \
         export -f 'foo=bar' 2> {error_path}; echo equals_status:$? >> {output_path}; \
         function /bin/echo {{ echo slash; }}; /bin/echo >> {output_path}; \
         export -f '/bin/echo' 2>> {error_path}; echo slash_status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "equals\nequals_status:1\nslash\nslash_status:1\n"
    );
    let errors = fs::read_to_string(error_path).unwrap();
    assert!(errors.contains("export: foo=bar: cannot export"));
    assert!(errors.contains("export: /bin/echo: cannot export"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_export_nf_unmarks_exported_function() {
    let output_path = "target/rubash-export-nf-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("foo() {{ :; }}; export -f foo; export -nf foo; declare -xF > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unset_f_clears_exported_function_attribute() {
    let output_path = "target/rubash-unset-exported-function-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "foo() {{ :; }}; export -f foo; unset -f foo; foo() {{ :; }}; declare -xF foo > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_readonly_f_protects_function_from_unset_and_redefine() {
    let output_path = "target/rubash-readonly-function-output.txt";
    let error_path = "target/rubash-readonly-function-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "foo() {{ echo one; }}; readonly -f foo; echo readonly:$? > {output_path}; \
         unset -f foo 2> {error_path}; echo unset:$? >> {output_path}; \
         foo() {{ echo two; }}; echo redefine:$? >> {output_path}; foo >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "readonly:0\nunset:1\nredefine:1\none\n"
    );
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("unset: foo: cannot unset: readonly function"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_readonly_function_redefine_with_redirect_preserves_failure_status() {
    let status_path = "target/rubash-readonly-function-redirect-status.txt";
    let error_path = "target/rubash-readonly-function-redirect-error.txt";
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "foo() {{ :; }}; readonly -f foo; \
         foo() {{ echo new; }} 2> {error_path}; echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert!(!std::path::Path::new(error_path).exists());
    let _ = fs::remove_file(status_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_readonly_f_missing_function_reports_error() {
    let error_path = "target/rubash-readonly-missing-function-error.txt";
    let status_path = "target/rubash-readonly-missing-function-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("readonly -f missing 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("readonly: missing: not a function"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_declare_fr_marks_function_readonly() {
    let output_path = "target/rubash-declare-readonly-function-output.txt";
    let error_path = "target/rubash-declare-readonly-function-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "foo() {{ echo one; }}; declare -fr foo; echo declare:$? > {output_path}; \
         unset -f foo 2> {error_path}; echo unset:$? >> {output_path}; \
         foo() {{ echo two; }}; echo redefine:$? >> {output_path}; foo >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "declare:0\nunset:1\nredefine:1\none\n"
    );
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("unset: foo: cannot unset: readonly function"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_declare_fx_exports_function_to_child_rubash() {
    let output_path = target_test_path("rubash-declare-export-function-child-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let input =
        format!("foo() {{ echo child; }}; declare -fx foo; {rubash} -c foo > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "child\n");
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_declare_fx_exports_nested_function_body_to_child_rubash() {
    let output_path = target_test_path("rubash-declare-export-nested-function-child-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let rubash = shell_test_path(std::path::Path::new(env!("CARGO_BIN_EXE_rubash")));
    let input = format!(
        "outer() {{ inner() {{ echo nested; }}; inner; }}; \
         declare -fx outer; {rubash} -c outer > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "nested\n");
    let _ = fs::remove_file(&output_path);
}
