use super::super::*;
use std::fs;

#[test]
fn test_local_outside_function_reports_error() {
    let error_path = "target/rubash-local-outside-error.txt";
    let status_path = "target/rubash-local-outside-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("local x=1 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    assert!(fs::read_to_string(error_path)
        .unwrap()
        .contains("local: can only be used in a function"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_local_assignment_is_restored_after_function() {
    let output_path = "target/rubash-local-restore-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "x=global; function f {{ local x=local; echo in:$x > {output_path}; }}; \
         f; echo out:$x >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "in:local\nout:global\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_nested_unset_removes_outer_local_and_preserves_global_assignment() {
    let output_path = "target/rubash-nested-unset-local-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "inner() {{ unset res; echo inner:${{res-res unset}} >> {output_path}; res[0]=X; res[1]=Y; }}; \
         outer() {{ local res=; inner; echo outer:${{res[@]}} >> {output_path}; }}; \
         echo main:${{res-unset}} > {output_path}; outer; echo main:${{res-unset}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "main:unset\ninner:res unset\nouter:X Y\nmain:X\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parenthesized_scalar_assignment_remains_scalar() {
    let output_path = "target/rubash-parenthesized-scalar-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("pattern='([a-z]+)([0-9]+)'; echo \"$pattern\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "([a-z]+)([0-9]+)\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_local_assignment_expands_parameter_value() {
    let output_path = "target/rubash-local-param-expansion-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() {{ [[ '1.000' =~ ^[-]?([0-9]*)\\.([0-9]+)$ ]]; local integerPart=${{BASH_REMATCH[1]:-0}} fractionalPart=${{BASH_REMATCH[2]}}; echo \"$integerPart/$fractionalPart\" > {output_path}; }}; f"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1/000\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_localvar_inherit_controls_unassigned_local_value() {
    let output_path = "target/rubash-localvar-inherit-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "x=global; f() {{ local x; printf 'off:<%s>\\n' \"$x\" > {output_path}; }}; f; \
         shopt -s localvar_inherit; g() {{ local x; printf 'on:<%s>\\n' \"$x\" >> {output_path}; }}; g; \
         printf 'out:<%s>\\n' \"$x\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "off:<>\non:<global>\nout:<global>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_local_new_variable_is_unset_after_function() {
    let output_path = "target/rubash-local-unset-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "function f {{ local -x RUBASH_LOCAL_TEMP=2; echo in:$RUBASH_LOCAL_TEMP > {output_path}; }}; \
         f; echo out:${{RUBASH_LOCAL_TEMP-unset}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "in:2\nout:unset\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_local_invalid_option_reports_usage() {
    let error_path = "target/rubash-local-invalid-error.txt";
    let status_path = "target/rubash-local-invalid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("function f {{ local -z 2> {error_path}; echo $? > {status_path}; }}; f");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("local: -z: invalid option"));
    assert!(error.contains("local: usage: local [option] name[=value] ..."));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_local_case_conversion_attributes_apply_to_assignments() {
    let output_path = "target/rubash-local-case-attrs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "function f {{ local -u up=abc; echo up1:$up > {output_path}; up=def; echo up2:$up >> {output_path}; \
         local -l low=ABC; echo low1:$low >> {output_path}; low=XYZ; echo low2:$low >> {output_path}; }}; \
         f; echo out:${{up-unset}}:${{low-unset}} >> {output_path}; \
         up=abc; low=XYZ; echo after:$up:$low >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "up1:ABC\nup2:DEF\nlow1:abc\nlow2:xyz\nout:unset:unset\nafter:abc:XYZ\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_local_plus_attributes_clear_conversion_and_integer_flags() {
    let output_path = "target/rubash-local-plus-attrs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "function f {{ local -u up=abc; local +u up; up=def; echo up:$up > {output_path}; \
         local -i n=2+3; local +i n; n=2+3; echo n:$n >> {output_path}; }}; f"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "up:def\nn:2+3\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_inside_function_is_local_by_default() {
    let output_path = target_test_path("rubash-function-declare-local-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "unset x n t; \
         f() {{ declare x=local; declare -i n=2+3; typeset t=value; \
                echo in:$x:$n:$t > {shell_output_path}; }}; \
         f; echo out:${{x-unset}}:${{n-unset}}:${{t-unset}} >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "in:local:5:value\nout:unset:unset:unset\n"
    );
    let _ = fs::remove_file(output_path);
}
