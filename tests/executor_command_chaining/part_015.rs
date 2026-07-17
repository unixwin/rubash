use super::super::*;
use std::fs;

#[test]
fn test_compound_array_assignment_expands_glob_matches_as_values() {
    let dir_path = target_test_path("rubash-array-glob-values");
    let output_path = target_test_path("rubash-array-glob-values-output.txt");
    let shell_dir_path = shell_test_path(&dir_path);
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_dir_all(&dir_path);
    let _ = fs::remove_file(&output_path);
    fs::create_dir_all(&dir_path).unwrap();
    let old_cwd = std::env::current_dir().unwrap();
    let input = format!(
        "cd {shell_dir_path}; touch '[3]=abcde' r s t u v; \
         x=(*); echo \"${{x[3]}}\" > {shell_output_path}; echo \"${{x[@]}}\" >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    std::env::set_current_dir(old_cwd).unwrap();
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "t\n[3]=abcde r s t u v\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(dir_path);
}

#[test]
fn test_quoted_expansions_suppress_pathname_expansion() {
    let dir_path = target_test_path("rubash-quoted-expansion-glob");
    let output_path = target_test_path("rubash-quoted-expansion-glob-output.txt");
    let shell_dir_path = shell_test_path(&dir_path);
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_dir_all(&dir_path);
    let _ = fs::remove_file(&output_path);
    fs::create_dir_all(&dir_path).unwrap();
    let old_cwd = std::env::current_dir().unwrap();
    let input = format!(
        "cd {shell_dir_path}; touch a.rs b.rs c.txt; \
         v='*.rs'; printf 'quoted<%s>\\n' \"$v\" > {shell_output_path}; \
         printf 'unquoted<%s>\\n' $v >> {shell_output_path}; \
         printf 'comsub<%s>\\n' \"$(printf '%s\\n' '*.rs')\" >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    std::env::set_current_dir(old_cwd).unwrap();
    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "quoted<*.rs>\nunquoted<a.rs>\nunquoted<b.rs>\ncomsub<*.rs>\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(dir_path);
}

#[test]
fn test_indexed_array_quoted_variable_subscript_evaluates_arithmetic() {
    let output_path = target_test_path("rubash-array-quoted-var-subscript-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input =
        format!("TOOLKIT=(1 2 3); ARRAY=1; echo ${{TOOLKIT[\"$ARRAY\"]}} > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_indexed_array_assignment_preserves_explicit_indices() {
    let output_path = "target/rubash-declare-indexed-array-sparse-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -a arr=([2]=two [0]=zero middle); declare -a arr+=([5]=five tail); \
         printf '%s / %s\\n' \"${{!arr[*]}}\" \"${{arr[*]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0 1 2 5 6 / zero middle two five tail\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_invalid_array_names_do_not_panic() {
    let output_path = "target/rubash-declare-invalid-array-names-output.txt";
    let sink_path = "target/rubash-declare-invalid-array-names-sink.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(sink_path);
    let input = format!(
        "declare -r []=asdf > {sink_path}; declare -r a[]=asdf >> {sink_path}; \
         echo survived > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "survived\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(sink_path);
}

#[test]
fn test_declare_rejects_indexed_to_assoc_conversion() {
    let output_path = target_test_path("rubash-declare-indexed-to-assoc-output.txt");
    let error_path = target_test_path("rubash-declare-indexed-to-assoc-error.txt");
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    let input = format!(
        "declare -a arr=(); declare -A arr=() 2> {shell_error_path}; \
         echo status:$? > {shell_output_path}; declare -p arr >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "status:1\ndeclare -a arr=()\n"
    );
    assert!(fs::read_to_string(&error_path)
        .unwrap()
        .contains("declare: arr: cannot convert indexed to associative array"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_declare_rejects_assoc_to_indexed_conversion() {
    let output_path = target_test_path("rubash-declare-assoc-to-indexed-output.txt");
    let error_path = target_test_path("rubash-declare-assoc-to-indexed-error.txt");
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    let input = format!(
        "declare -A assoc; declare -a assoc=() 2> {shell_error_path}; \
         echo status:$? > {shell_output_path}; declare -p assoc >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "status:1\ndeclare -A assoc\n"
    );
    assert!(fs::read_to_string(&error_path)
        .unwrap()
        .contains("declare: assoc: cannot convert associative to indexed array"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_declare_readonly_assoc_array_prints_combined_attrs() {
    let output_path = target_test_path("rubash-declare-readonly-assoc-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!("declare -rA FOOBAR=([foo]=bar); declare -p FOOBAR > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "declare -Ar FOOBAR=([foo]=\"bar\" )\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_indexed_array_arithmetic_subscript_names_default_to_zero() {
    let output_path = target_test_path("rubash-declare-indexed-named-subscript-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input =
        format!("declare -ra FOOBAR2=([foo]=bar); declare -p FOOBAR2 > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "declare -ar FOOBAR2=([0]=\"bar\")\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_indexed_array_assignment_resolves_negative_indices() {
    let output_path = "target/rubash-declare-indexed-array-negative-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -a arr=([2]=two [5]=five); declare -a arr+=([-1]=FIVE [-4]=TWO); \
         printf '%s / %s\\n' \"${{!arr[*]}}\" \"${{arr[*]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 5 / TWO FIVE\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_integer_array_assignment_preserves_indices() {
    let output_path = "target/rubash-declare-integer-array-sparse-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -ai arr=([2]=1+2 [0]=3+4 middle); declare -ai arr+=([2]+=5 [5]=2+6 tail); \
         printf '%s / %s\\n' \"${{!arr[*]}}\" \"${{arr[*]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0 1 2 5 6 / 7 0 8 8 0\n"
    );
    let _ = fs::remove_file(output_path);
}
