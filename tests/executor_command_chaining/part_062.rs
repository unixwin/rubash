use super::super::*;
use std::fs;

#[test]
fn test_parameter_equals_assigns_default_only_when_unset() {
    let output_path = "target/rubash-param-equals-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset v; : ${{v=default}}; echo unset:$v > {output_path}; v=; : ${{v=default}}; echo empty:$v >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "unset:default\nempty:\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_equals_assigns_before_regular_command() {
    let output_path = "target/rubash-param-equals-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("unset v; echo ${{v=default}} $v > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "default default\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_element_parameter_assignment_writes_element() {
    let output_path = "target/rubash-array-element-param-assign-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(); : ${{arr[1]:=bee}}; : ${{arr[1]:=skip}}; \
         : ${{arr[0]=}}; : ${{arr[0]=skip}}; \
         declare -A assoc; : ${{assoc[key]=ant}}; : ${{assoc[key]=skip}}; \
         printf '<%s>|<%s>|<%s>\\n' \"${{arr[0]}}\" \"${{arr[1]}}\" \"${{assoc[key]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "<>|<bee>|<ant>\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_assignment_expansion_assigns_inside_quoted_words() {
    let output_path = "target/rubash-param-assign-quoted-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset v w; v=; printf '<%s>:%s\\n' \"${{v:=default}}\" \"$v\" > {output_path}; \
         printf '<%s>:%s\\n' \"x${{w=word}}y\" \"$w\" >> {output_path}; \
         unset cmd; ${{cmd:=echo}} command-position:$cmd >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "<default>:default\n<xwordy>:word\ncommand-position:echo\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_assignment_expansion_assigns_inside_assignment_values() {
    let output_path = "target/rubash-param-assign-rhs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset v x w y; x=${{v:=default}}; printf 'x=<%s> v=<%s>\\n' \"$x\" \"$v\" > {output_path}; \
         y=pre${{w=word}}post; printf 'y=<%s> w=<%s>\\n' \"$y\" \"$w\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "x=<default> v=<default>\ny=<prewordpost> w=<word>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_nested_parameter_assignment_expansion_assigns_inner_word() {
    let output_path = "target/rubash-param-assign-nested-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset a b; a=set; printf 'alt=<%s> b=<%s>\\n' \"${{a:+${{b:=bee}}}}\" \"$b\" > {output_path}; \
         unset a b; printf 'def=<%s> b=<%s>\\n' \"${{a:-${{b:=bee}}}}\" \"$b\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "alt=<bee> b=<bee>\ndef=<bee> b=<bee>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_nested_parameter_assignment_expansion_assigns_outer_rhs() {
    let output_path = "target/rubash-param-assign-nested-rhs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset a b; printf 'outer=<%s> a=<%s> b=<%s>\\n' \"${{a:=${{b:=bee}}}}\" \"$a\" \"$b\" > {output_path}; \
         unset a b; a=old; printf 'skip=<%s> a=<%s> b=<%s>\\n' \"${{a:=${{b:=bee}}}}\" \"$a\" \"${{b-unset}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "outer=<bee> a=<bee> b=<bee>\nskip=<old> a=<old> b=<unset>\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_parameter_assignment_expansion_rejects_readonly_targets() {
    let output_path = target_test_path("rubash-param-assign-readonly-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    std::env::remove_var("RUBASH_PARAM_ASSIGN_RO");
    let input = format!(
        "unset RUBASH_PARAM_ASSIGN_RO; readonly RUBASH_PARAM_ASSIGN_RO; \
         printf '<%s>\\n' \"${{RUBASH_PARAM_ASSIGN_RO:=new}}\" > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
    assert_eq!(executor.last_exit_code(), 1);
    assert!(!output_path.exists());
    std::env::remove_var("RUBASH_PARAM_ASSIGN_RO");
}

#[test]
fn test_parameter_assignment_expansion_reports_readonly_nameref_target() {
    let output_path = target_test_path("rubash-param-assign-readonly-nameref-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    std::env::remove_var("RUBASH_PARAM_ASSIGN_RO_TARGET");
    std::env::remove_var("RUBASH_PARAM_ASSIGN_RO_REF");
    let input = format!(
        "unset RUBASH_PARAM_ASSIGN_RO_TARGET RUBASH_PARAM_ASSIGN_RO_REF; \
         readonly RUBASH_PARAM_ASSIGN_RO_TARGET; \
         declare -n RUBASH_PARAM_ASSIGN_RO_REF=RUBASH_PARAM_ASSIGN_RO_TARGET; \
         printf '<%s>\\n' \"${{RUBASH_PARAM_ASSIGN_RO_REF:=new}}\" > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
    assert_eq!(executor.last_exit_code(), 1);
    assert!(!output_path.exists());
    std::env::remove_var("RUBASH_PARAM_ASSIGN_RO_TARGET");
    std::env::remove_var("RUBASH_PARAM_ASSIGN_RO_REF");
}

#[test]
fn test_parameter_assignment_expansion_rejects_positional_parameters() {
    let output_path = target_test_path("rubash-param-assign-positional-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!("set --; printf '<%s>\\n' \"${{1:=default}}\" > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
    assert_eq!(executor.last_exit_code(), 1);
    assert!(!output_path.exists());
}

#[test]
fn test_parameter_assignment_expansion_uses_existing_positional_and_special_values() {
    let output_path = "target/rubash-param-assign-special-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -- ok; printf 'pos:<%s>\\n' \"${{1:=default}}\" > {output_path}; \
         set -- ''; \
         printf 'empty:<%s>\\n' \"${{1=default}}\" >> {output_path}; \
         printf 'status:<%s>\\n' \"${{?:=default}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "pos:<ok>\nempty:<>\nstatus:<0>\n"
    );
    let _ = fs::remove_file(output_path);
}
