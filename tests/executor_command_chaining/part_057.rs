use super::super::*;
use std::fs;

#[test]
fn test_declare_g_inside_function_unsets_global_attributes() {
    let output_path = target_test_path("rubash-function-declare-global-unset-attrs-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "declare -i n=5; declare -u x=ABC; declare -x y=exported; \
         f() {{ declare -g +i n; n=2+3; declare -g +u x; x=MiXeD; declare -g +x y; }}; \
         f; declare -p n x y > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "declare -- n=\"2+3\"\ndeclare -- x=\"MiXeD\"\ndeclare -- y=\"exported\"\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_g_export_unset_global_behind_local_keeps_value_unset() {
    let output_path = target_test_path("rubash-function-declare-global-export-unset-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "unset x; f() {{ local x=local; declare -g -x x; \
                echo in:${{x-unset}} > {shell_output_path}; }}; \
         f; declare -p x >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "in:local\ndeclare -x x\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_readonly_attributes_persist_but_local_attrs_restore() {
    let output_path = target_test_path("rubash-function-readonly-attrs-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "a=(outside); n=1; \
         f() {{ readonly a=(1); readonly n=4; local -x temp=local; }}; \
         f; declare -p a n > {shell_output_path}; \
         declare -p temp 2>/dev/null || echo temp-unset >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "declare -ar a=([0]=\"1\")\ndeclare -r n=\"4\"\ntemp-unset\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_getopts_parses_explicit_option_argument() {
    let output_path = "target/rubash-getopts-explicit-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("getopts a: store -a aoptval; echo $?:$store:$OPTARG:$OPTIND > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0:a:aoptval:3\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_getopts_walks_clustered_options() {
    let output_path = "target/rubash-getopts-cluster-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "getopts ab opt -ab; echo $opt:$OPTIND:$? > {output_path}; getopts ab opt -ab; echo $opt:$OPTIND:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "a:1:0\nb:2:0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_getopts_reports_missing_argument_modes() {
    let output_path = "target/rubash-getopts-missing-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "OPTERR=0; getopts a: opt -a; echo $?:$opt:${{OPTARG-unset}}:$OPTIND > {output_path}; \
         OPTIND=1; getopts :a: opt -a; echo $?:$opt:$OPTARG:$OPTIND >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0:?:unset:2\n0:::a:2\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_getopts_invalid_builtin_option_redirects_stderr() {
    let error_path = "target/rubash-getopts-invalid-error.txt";
    let status_path = "target/rubash-getopts-invalid-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("getopts -a opts name 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("getopts: -a: invalid option\n"));
    assert!(error.contains("getopts: usage: getopts optstring name [arg ...]\n"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_builtin_and_command_getopts_update_shell_state() {
    let output_path = "target/rubash-getopts-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "builtin getopts a opt -a; echo builtin:$?:$opt:$OPTIND > {output_path}; \
         OPTIND=1; command getopts a opt -a; echo command:$?:$opt:$OPTIND >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "builtin:0:a:2\ncommand:0:a:2\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shift_too_many_fails_without_changing_positional_params() {
    let output_path = "target/rubash-shift-too-many-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function s {{ shift 3; echo $? $# $1 > {output_path}; }}; s one two");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1 2 one\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shift_non_numeric_fails_without_changing_positional_params() {
    let output_path = "target/rubash-shift-nonnumeric-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function s {{ shift x; echo $? $# $1 > {output_path}; }}; s one two");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1 2 one\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shift_negative_fails_without_changing_positional_params() {
    let output_path = "target/rubash-shift-negative-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function s {{ shift -1; echo $? $# $1 > {output_path}; }}; s one two");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1 2 one\n");
    let _ = fs::remove_file(output_path);
}
