use super::super::*;
use std::fs;

#[test]
fn test_conditional_shell_option_unary() {
    let output_path = "target/rubash-conditional-shell-option-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -o errexit; [[ -o errexit ]]; echo $? > {output_path}; set +o errexit; [[ -o errexit ]]; echo $? >> {output_path}; [[ -o no_such_option ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_short_errexit_updates_shell_option() {
    let output_path = "target/rubash-short-errexit-option-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -e; [[ -o errexit ]]; echo $? > {output_path}; set +e; [[ -o errexit ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_set_o_errexit_exits_on_failed_command() {
    let output_path = "target/rubash-set-o-errexit-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("set -o errexit; false; echo after > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(matches!(result, Err(ExecuteError::ExitCode(1))));
    assert_eq!(executor.last_exit_code(), 1);
    assert!(!std::path::Path::new(output_path).exists());
}

#[test]
fn test_test_shell_option_unary() {
    let output_path = "target/rubash-test-shell-option-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "set -o errexit; test -o errexit; echo $? > {output_path}; set +o errexit; test -o errexit; echo $? >> {output_path}; [ -o no_such_option ]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_conditional_numeric_checks_evaluate_arithmetic_expressions() {
    let output_path = "target/rubash-conditional-arithmetic-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=3; [[ n+2*4 -eq 11 ]]; echo $? > {output_path}; [[ $n*2 -ge 6 ]]; echo $? >> {output_path}; [[ -5+2 -lt 0 ]]; echo $? >> {output_path}; [[ n/0 -eq 0 ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n0\n0\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_status_uses_expression_value() {
    let output_path = "target/rubash-arithmetic-command-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=3; (( 0 )); echo $? > {output_path}; (( n + 1 )); echo $? >> {output_path}; (( n - 3 )); echo $? >> {output_path}; (( n * 2 )); echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n1\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_redirect_creates_output_file() {
    let output_path = "target/rubash-arithmetic-command-redirect-output.txt";
    let status_path = "target/rubash-arithmetic-command-redirect-status.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
    let input = format!(
        "(( 1 )) > {output_path}; echo true:$? > {status_path}; \
         (( 0 )) >> {output_path}; echo false:$? >> {status_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "");
    assert_eq!(
        fs::read_to_string(status_path).unwrap(),
        "true:0\nfalse:1\n"
    );
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_and_brace_group_short_circuits_and_returns() {
    let output_path = "target/rubash-and-brace-group-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "f() {{ (( ($# < 2) || ($# > 3) )) && {{ echo bad >> {output_path}; return 2; }}; echo ok >> {output_path}; }}; \
         f a b; echo status:$? >> {output_path}; f a; echo status:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "ok\nstatus:0\nbad\nstatus:2\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_expansion_evaluates_expressions() {
    let output_path = "target/rubash-arithmetic-expansion-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=5; echo $((n+2*3)) > {output_path}; echo $((16#ff-250)) >> {output_path}; echo $((n>4?7:9)) >> {output_path}; echo $((2**3**2)) >> {output_path}; echo pre$((1+(2*3)))post >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "11\n5\n7\n512\npre7post\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_expansion_applies_side_effects() {
    let output_path = "target/rubash-arithmetic-expansion-side-effects-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=1; echo $((n++)) $n > {output_path}; echo pre$((n+=4))post $n >> {output_path}; echo $((a=b=3)) $a $b >> {output_path}; x=$((n++)); echo $x $n >> {output_path}; echo $((0 && (n+=99))) $n >> {output_path}; echo $((1 || (n+=99))) $n >> {output_path}; echo $((1 && (n+=2))) $n >> {output_path}; echo $((0 || (n+=3))) $n >> {output_path}; echo $((1 ? (n+=4) : (n+=99))) $n >> {output_path}; echo $((0 ? (n+=99) : (n+=5))) $n >> {output_path}; echo $((n++ + 1)) $n >> {output_path}; echo $((++n + 1)) $n >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "1 2\npre6post 6\n3 3 3\n6 7\n0 7\n1 7\n1 9\n1 12\n16 16\n21 21\n22 22\n24 23\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_legacy_bracket_arithmetic_expansion_evaluates() {
    let output_path = "target/rubash-legacy-bracket-arithmetic-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(4 7); i=1; n=2; \
         echo $[1 + 2] pre$[n * 3]post $[arr[i] + 5] > {output_path}; \
         echo $[n++] $n >> {output_path}; \
         x=$[n += 4]; echo $x $n >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "3 pre6post 12\n2 3\n7 7\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_updates_variables() {
    let output_path = "target/rubash-arithmetic-command-updates-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=0; (( n++ )); echo $? $n > {output_path}; (( ++n )); echo $? $n >> {output_path}; (( n += 3 )); echo $? $n >> {output_path}; (( n = 0 )); echo $? $n >> {output_path}; (( n /= 0 )); echo $? $n >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "1 1\n0 2\n0 5\n1 0\n1 0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_assignments_evaluate_rhs_recursively() {
    let output_path = "target/rubash-arithmetic-recursive-assign-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "a=0; b=0; (( a = b = 3 )); echo $? $a $b > {output_path}; let 'a+=b=4'; echo $? $a $b >> {output_path}; (( z = a = b = 0 )); echo $? $a $b $z >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0 3 3\n0 7 4\n1 0 0 0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_variables_evaluate_recursively() {
    let output_path = "target/rubash-arithmetic-recursive-vars-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "x='1+2'; echo $((x)) > {output_path}; x=y; y=5; echo $((x)) >> {output_path}; n=1; x='n+=2'; echo $((x)) $n >> {output_path}; n=1; x='n+=2'; [[ x -eq 3 ]]; echo $? $n >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "3\n5\n3 3\n0 3\n");
    let _ = fs::remove_file(output_path);
}
