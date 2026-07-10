use super::super::*;
use std::fs;

#[test]
fn test_let_bitwise_assignment_operators() {
    let output_path = "target/rubash-let-bitwise-assign-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=12; let 'n&=10'; echo $? $n > {output_path}; let 'n|=1'; echo $? $n >> {output_path}; let 'n^=3'; echo $? $n >> {output_path}; let 'n<<=2'; echo $? $n >> {output_path}; let 'n>>=1'; echo $? $n >> {output_path}; let 'n&=0'; echo $? $n >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0 8\n0 9\n0 10\n0 40\n0 20\n1 0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_bitwise_assignment_operators() {
    let output_path = "target/rubash-arithmetic-command-bitwise-assign-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=12; (( n &= 10 )); echo $? $n > {output_path}; (( n |= 1 )); echo $? $n >> {output_path}; (( n ^= 3 )); echo $? $n >> {output_path}; (( n <<= 2 )); echo $? $n >> {output_path}; (( n >>= 1 )); echo $? $n >> {output_path}; (( n &= 0 )); echo $? $n >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0 8\n0 9\n0 10\n0 40\n0 20\n1 0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_exponentiation_operators() {
    let output_path = "target/rubash-arithmetic-command-exponent-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=2; (( n ** 3 )); echo $? > {output_path}; [[ 2**3**2 -eq 512 ]]; echo $? >> {output_path}; (( n **= 4 )); echo $? $n >> {output_path}; (( 2 ** -1 )); echo $? >> {output_path}; (( 2 ** 200 )); echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n0\n0 16\n1\n1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_uses_bash_int64_overflow_semantics() {
    let output_path = "target/rubash-arithmetic-overflow-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "echo $((2**63)) > {output_path}; \
         [[ 2**63 -lt 2**63-1 ]]; echo $? >> {output_path}; \
         echo $((9223372036854775807 + 1)) >> {output_path}; \
         echo $((1 << 64)) >> {output_path}; \
         echo $((-1 >> 1)) >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "-9223372036854775808\n0\n-9223372036854775808\n1\n-1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_expansion_expands_braced_parameter_operations() {
    let output_path = "target/rubash-arithmetic-param-op-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "number=166666666666666666; echo $(( ${{number//[^-]}}10#${{number//[^0-9]}} + 1 )) > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "166666666666666667\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shellmath_decimal_divide_script_path() {
    let output_path = target_test_path("rubash-shellmath-divide-output.txt");
    let _ = fs::remove_file(&output_path);
    let shell_output_path = shell_test_path(&output_path);
    let shellmath_path = shell_test_path(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("bash")
            .join("examples")
            .join("shellmath")
            .join("shellmath.sh"),
    );
    let input = format!(
        "source {shellmath_path}; __shellmath_isOptimized=1; _shellmath_precalc; _shellmath_divide 0.500 3; \
         _shellmath_getReturnValue v; echo \"$v\" > {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "0.166666666666666667\n"
    );
    let _ = fs::remove_file(&output_path);
}

#[test]
fn test_arithmetic_command_based_integer_constants() {
    let output_path = "target/rubash-arithmetic-command-bases-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "(( 2#101 - 5 )); echo $? > {output_path}; [[ 16#FF -eq 255 ]]; echo $? >> {output_path}; [[ 0x10 -eq 16 ]]; echo $? >> {output_path}; [[ 010 -eq 8 ]]; echo $? >> {output_path}; [[ 64#_ -eq 63 ]]; echo $? >> {output_path}; (( 8#9 )); echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "1\n0\n0\n0\n0\n1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_conditional_operator() {
    let output_path = "target/rubash-arithmetic-command-conditional-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=3; (( n > 2 ? n - 3 : 9 )); echo $? > {output_path}; (( n < 2 ? 0 : n + 1 )); echo $? >> {output_path}; [[ n==3?7:9 -eq 7 ]]; echo $? >> {output_path}; [[ n==4?7:9 -eq 9 ]]; echo $? >> {output_path}; [[ 0?1:0?2:3 -eq 3 ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n0\n0\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_conditional_operator_is_lazy() {
    let output_path = "target/rubash-arithmetic-conditional-lazy-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "(( 1 ? 4 : 1 / 0 )); echo $? > {output_path}; (( 0 ? 1 / 0 : 4 )); echo $? >> {output_path}; (( 1 ? 0 : 1 / 0 )); echo $? >> {output_path}; (( 0 ? 1 / 0 : 0 )); echo $? >> {output_path}; (( 1 ? 1 ? 5 : 1 / 0 : 1 / 0 )); echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n0\n1\n1\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_drives_if_conditions() {
    let output_path = "target/rubash-arithmetic-if-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=1; if (( n )); then echo yes > {output_path}; else echo no > {output_path}; fi; if (( n - 1 )); then echo bad >> {output_path}; elif (( n + 1 )); then echo elif >> {output_path}; else echo bad >> {output_path}; fi; if (( n++ )); then echo $n >> {output_path}; fi"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "yes\nelif\n2\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_drives_loop_conditions() {
    let output_path = "target/rubash-arithmetic-loop-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=0; while (( n < 3 )); do echo $n >> {output_path}; (( n++ )); done; until (( n == 5 )); do (( n++ )); done; echo $n >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n2\n5\n");
    let _ = fs::remove_file(output_path);
}
