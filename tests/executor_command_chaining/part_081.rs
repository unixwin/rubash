use super::super::*;
use std::fs;

#[test]
fn test_seconds_matches_between_parameter_and_arithmetic_expansion() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-seconds-arithmetic-output.txt";
    let _ = fs::remove_file(output_path);
    // Assigning SECONDS shifts its reference point, making the value
    // deterministic for a freshly started shell (elapsed 0 => value 10).
    let input = format!(
        "SECONDS=10; a=$SECONDS; b=$((SECONDS)); printf '%s:%s\\n' \"$a\" \"$b\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let (parameter, arithmetic) = output.trim_end().split_once(':').unwrap();
    assert_eq!(parameter, "10");
    assert_eq!(arithmetic, "10");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_seconds_used_in_arithmetic_expression() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-seconds-arithmetic-sum-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("SECONDS=10; echo $((SECONDS - 7)) > {output_path}");
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
fn test_seconds_assignment_inside_arithmetic_command_resets_base() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-seconds-arithmetic-assign-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("((SECONDS = 10)); a=$SECONDS; b=$((SECONDS)); printf '%s:%s\\n' \"$a\" \"$b\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    let (parameter, arithmetic) = output.trim_end().split_once(':').unwrap();
    assert_eq!(parameter, "10");
    assert_eq!(arithmetic, "10");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_seconds_is_bound_under_nounset_in_arithmetic() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-seconds-arithmetic-nounset-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("set -u; SECONDS=10; echo $((SECONDS)) > {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(fs::read_to_string(output_path).unwrap(), "10\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_epochseconds_usable_in_arithmetic() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-epochseconds-arithmetic-output.txt";
    let _ = fs::remove_file(output_path);
    // The wall clock is non-decreasing, so a captured $EPOCHSECONDS can
    // never exceed a later $((EPOCHSECONDS)) read.
    let input = format!("a=$EPOCHSECONDS; echo $((a <= EPOCHSECONDS)) > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_lineno_usable_in_arithmetic() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-lineno-arithmetic-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("echo $((LINENO)) > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let value = fs::read_to_string(output_path)
        .unwrap()
        .trim_end()
        .parse::<i128>()
        .unwrap();
    assert!(value >= 1);
    let _ = fs::remove_file(output_path);
}
