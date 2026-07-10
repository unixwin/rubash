use super::super::*;
use std::fs;

#[test]
fn test_declare_g_inside_function_assigns_global() {
    let output_path = target_test_path("rubash-function-declare-global-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "unset x; f() {{ declare -g x=global; echo in:$x > {shell_output_path}; }}; \
         f; echo out:$x >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "in:global\nout:global\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_g_inside_function_updates_global_behind_local() {
    let output_path = target_test_path("rubash-function-declare-global-behind-local-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "x=global; f() {{ local x=local; declare -g x=changed; \
                echo in:$x > {shell_output_path}; }}; \
         f; echo out:$x >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "in:local\nout:changed\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_g_expanded_compound_assignment_updates_global_behind_local() {
    let output_path =
        target_test_path("rubash-function-declare-global-expanded-compound-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "unset x; f() {{ local -a x; name=x; declare -ga \"$name=( one two )\"; \
                echo in:${{x[@]}} > {shell_output_path}; }}; \
         f; echo out:${{x[@]}} >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "in:\nout:one two\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_g_nested_function_updates_global_behind_outer_local() {
    let output_path = target_test_path("rubash-function-declare-global-nested-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "unset x y; \
         f() {{ local x; local -a y; g; echo f:$x:${{y[@]}} >> {shell_output_path}; }}; \
         g() {{ name=y; declare -g x=scalar; declare -ga \"$name=( one two )\"; \
                echo g:$x:${{y[@]}} > {shell_output_path}; }}; \
         f; echo out:$x:${{y[@]}} >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "g::\nf::\nout:scalar:one two\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_local_p_prints_without_reinitializing_local() {
    let output_path = target_test_path("rubash-local-p-preserves-attrs-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "f() {{ local -a arr=(); local -r arr=0; local -p arr > {shell_output_path}; \
                local -i n=0; local -r n=1; local -p n >> {shell_output_path}; }}; f"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "declare -ar arr=([0]=\"0\")\ndeclare -ir n=\"1\"\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_local_without_assignment_unsets_shell_value_but_preserves_export_env() {
    let output_path = target_test_path("rubash-local-export-env-output.txt");
    let script_path = target_test_path("rubash-local-export-env-child.sh");
    let shell_output_path = shell_test_path(&output_path);
    let shell_script_path = shell_test_path(&script_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&script_path);
    fs::write(
        &script_path,
        "printf '%s\\n' \"${RUBASH_LOCAL_ENV-unset}\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
    }
    let input = format!(
        "unset RUBASH_LOCAL_ENV; export RUBASH_LOCAL_ENV=abc; \
         f() {{ local RUBASH_LOCAL_ENV; echo local:${{RUBASH_LOCAL_ENV-unset}} > {shell_output_path}; \
                {shell_script_path} >> {shell_output_path}; }}; \
         f; echo outer:$RUBASH_LOCAL_ENV >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "local:unset\nabc\nouter:abc\n"
    );
    std::env::remove_var("RUBASH_LOCAL_ENV");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(script_path);
}

#[test]
fn test_local_readonly_assignment_reports_local_builtin_name() {
    let output_path = target_test_path("rubash-local-readonly-error-output.txt");
    let error_path = target_test_path("rubash-local-readonly-error-stderr.txt");
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    let input = format!(
        "readonly RUBASH_LOCAL_RO=outer; \
         f() {{ local RUBASH_LOCAL_RO=inner 2> {shell_error_path}; echo status:$? > {shell_output_path}; }}; f"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "status:1\n");
    assert!(fs::read_to_string(&error_path)
        .unwrap()
        .contains("local: RUBASH_LOCAL_RO: readonly variable"));
    std::env::remove_var("RUBASH_LOCAL_RO");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_local_compound_readonly_assignment_reports_underlying_error() {
    let output_path = target_test_path("rubash-local-compound-readonly-output.txt");
    let error_path = target_test_path("rubash-local-compound-readonly-stderr.txt");
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    let input = format!(
        "readonly RUBASH_LOCAL_COMPOUND_RO=outer; \
         f() {{ local RUBASH_LOCAL_COMPOUND_RO=(one two) 2> {shell_error_path}; \
                echo status:$? > {shell_output_path}; }}; f"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "status:1\n");
    let error = fs::read_to_string(&error_path).unwrap();
    assert!(error.contains("RUBASH_LOCAL_COMPOUND_RO: readonly variable"));
    assert!(error.contains("local: RUBASH_LOCAL_COMPOUND_RO: readonly variable"));
    std::env::remove_var("RUBASH_LOCAL_COMPOUND_RO");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_declare_g_inside_function_preserves_global_attributes_behind_locals() {
    let output_path = target_test_path("rubash-function-declare-global-attrs-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "n=1; x=global; y=global; \
         f() {{ declare -g -i n=2+3; local -l x=LOCAL; declare -g -u x=changed; \
                x=MiXeD; local y=local; declare -gx y=exported; \
                echo in:$x:$y > {shell_output_path}; }}; \
         f; declare -p n x y >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "in:mixed:local\ndeclare -i n=\"5\"\ndeclare -u x=\"CHANGED\"\ndeclare -x y=\"exported\"\n"
    );
    let _ = fs::remove_file(output_path);
}
