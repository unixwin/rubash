use super::super::*;
use std::fs;

#[test]
fn test_arithmetic_command_and_or_short_circuits() {
    let output_path = "target/rubash-arithmetic-and-or-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "x=0; ((x)) && ((y=1)); echo zero:${{y-unset}}:$? > {output_path}; x=1; ((x)) && ((y=2)); echo one:$y:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "zero:unset:1\none:2:0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_treats_newline_as_whitespace() {
    let output_path = "target/rubash-arithmetic-newline-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("(( value = 1000 /\n10 )); echo $value > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "100\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_if_condition_honors_arithmetic_and_or_list() {
    let output_path = "target/rubash-if-arith-and-or-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "if (( 1 )) && (( 0 )); then echo bad > {output_path}; else echo ok > {output_path}; fi"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_for_loop_break_continue_and_empty_test() {
    let output_path = "target/rubash-arithmetic-for-control-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "for (( i = 0; ; i++ )); do if (( i == 1 )); then continue; fi; echo $i >> {output_path}; if (( i == 3 )); then break; fi; done"
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
fn test_arithmetic_for_loop_exit_status_matches_body_or_zero_iterations() {
    let output_path = "target/rubash-arithmetic-for-status-output.txt";
    let _ = fs::remove_file(output_path);
    let loop_only_tokens = tokenize("for (( i = 0; i < 1; i++ )); do false; done");
    let loop_only_ast = parse(&loop_only_tokens);
    let mut loop_only_executor = Executor::new();
    let loop_only_result = loop_only_executor.execute_ast(&loop_only_ast);
    assert!(loop_only_result.is_ok());
    assert_eq!(loop_only_executor.last_exit_code(), 1);

    let input = format!(
        "for (( i = 0; i < 1; i++ )); do false; done; echo $? > {output_path}; false; for (( i = 0; i < 0; i++ )); do true; done; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_positional_count_expands() {
    let output_path = "target/rubash-function-count-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function argc {{ echo $# > {output_path}; }}; argc one two three");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "3\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_function_positional_star_expands() {
    let output_path = "target/rubash-function-star-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("function argv {{ echo $* > {output_path}; }}; argv one two three");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "one two three\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_capital_f_prints_function_names() {
    let output_path = "target/rubash-declare-F-output.txt";
    let status_path = "target/rubash-declare-F-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "foo() {{ echo hi; }}; bar() {{ :; }}; \
         declare -F foo missing bar > {output_path}; echo $? > {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "foo\nbar\n");
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_declare_capital_f_lists_all_functions() {
    let output_path = "target/rubash-declare-F-all-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("foo() {{ :; }}; bar() {{ :; }}; declare -F > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "declare -f bar\ndeclare -f foo\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_lower_f_redirects_function_definition() {
    let output_path = "target/rubash-declare-f-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("foo() {{ echo hi; }}; declare -f foo > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "foo () \n{ \n    echo hi\n}\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_lower_f_prints_compound_function_bodies() {
    let output_path = "target/rubash-declare-f-compound-bodies-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() for x in a; {{ echo $x; }}; \
         s() select y in b; {{ echo $y; break; }}; \
         c() case $1 in a) echo alpha ;; *) echo other ;; esac; \
         declare -f f s c > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("    for x in a; { echo $x; }"));
    assert!(output.contains("    select y in b; { echo $y; break; }"));
    assert!(output.contains("    case $1 in a) echo alpha ;; *) echo other ;; esac"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_lower_f_prints_nested_function_definition_body() {
    let output_path = "target/rubash-declare-f-nested-function-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "outer() {{ inner() {{ echo nested; }}; inner; }}; declare -f outer > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("    inner() { echo nested; }"));
    assert!(output.contains("    inner\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_prints_function_here_strings_like_bash() {
    let output_path = target_test_path("rubash-declare-function-herestr-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    let input = format!(
        "a=hot; b=damn; \
         f() {{ cat <<< \"abcde\"; cat <<< \"$a $b\"; cat <<< 'double\"quote'; }}; \
         g() {{ cat <<< \"$@\"; }}; \
         h() {{ cat <<< onetwothree; }}; \
         declare -f f g h > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(&output_path).unwrap();
    assert!(output.contains("    cat <<< \"abcde\";\n"));
    assert!(output.contains("    cat <<< \"$a $b\";\n"));
    assert!(output.contains("    cat <<< 'double\"quote'\n"));
    assert!(output.contains("    cat <<< \"$@\"\n"));
    assert!(output.contains("    cat <<< onetwothree\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_exported_functions_are_empty_without_function_exports() {
    let output_path = "target/rubash-declare-exported-functions-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("foo() {{ :; }}; declare -xF > {output_path}; declare -xf >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    let _ = fs::remove_file(output_path);
}
