use super::super::*;
use std::fs;

#[test]
fn test_export_and_readonly_nameref_apply_to_targets() {
    let output_path = target_test_path("rubash-nameref-setattr-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    for name in [
        "RUBASH_NAMEREF_EXPORT_TARGET",
        "RUBASH_NAMEREF_EXPORT_REF",
        "RUBASH_NAMEREF_READONLY_TARGET",
        "RUBASH_NAMEREF_READONLY_REF",
    ] {
        std::env::remove_var(name);
    }
    let input = format!(
        "RUBASH_NAMEREF_EXPORT_TARGET=value; RUBASH_NAMEREF_READONLY_TARGET=value; \
         declare -n RUBASH_NAMEREF_EXPORT_REF=RUBASH_NAMEREF_EXPORT_TARGET; \
         declare -n RUBASH_NAMEREF_READONLY_REF=RUBASH_NAMEREF_READONLY_TARGET; \
         export RUBASH_NAMEREF_EXPORT_REF=changed; readonly RUBASH_NAMEREF_READONLY_REF=locked; \
         declare -p RUBASH_NAMEREF_EXPORT_REF RUBASH_NAMEREF_EXPORT_TARGET RUBASH_NAMEREF_READONLY_REF RUBASH_NAMEREF_READONLY_TARGET > {shell_output_path}; \
         export -n RUBASH_NAMEREF_EXPORT_REF; declare -p RUBASH_NAMEREF_EXPORT_TARGET >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "declare -n RUBASH_NAMEREF_EXPORT_REF=\"RUBASH_NAMEREF_EXPORT_TARGET\"\ndeclare -x RUBASH_NAMEREF_EXPORT_TARGET=\"changed\"\ndeclare -n RUBASH_NAMEREF_READONLY_REF=\"RUBASH_NAMEREF_READONLY_TARGET\"\ndeclare -r RUBASH_NAMEREF_READONLY_TARGET=\"locked\"\ndeclare -- RUBASH_NAMEREF_EXPORT_TARGET=\"changed\"\n"
    );
    for name in [
        "RUBASH_NAMEREF_EXPORT_TARGET",
        "RUBASH_NAMEREF_EXPORT_REF",
        "RUBASH_NAMEREF_READONLY_TARGET",
        "RUBASH_NAMEREF_READONLY_REF",
    ] {
        std::env::remove_var(name);
    }
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_nameref_rejects_self_reference() {
    let output_path = target_test_path("rubash-nameref-self-reference-output.txt");
    let error_path = target_test_path("rubash-nameref-self-reference-error.txt");
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    std::env::remove_var("RUBASH_NAMEREF_SELF");
    let input = format!(
        "RUBASH_NAMEREF_SELF=old; declare -n RUBASH_NAMEREF_SELF=RUBASH_NAMEREF_SELF 2> {shell_error_path}; \
         echo status:$? > {shell_output_path}; declare -p RUBASH_NAMEREF_SELF >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "status:1\ndeclare -- RUBASH_NAMEREF_SELF=\"old\"\n"
    );
    let error = fs::read_to_string(&error_path).unwrap();
    assert!(error
        .contains("declare: RUBASH_NAMEREF_SELF: nameref variable self references not allowed"));
    std::env::remove_var("RUBASH_NAMEREF_SELF");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_declare_nameref_cycle_expands_as_unset_and_rejects_assignment() {
    let output_path = target_test_path("rubash-nameref-cycle-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    std::env::remove_var("RUBASH_NAMEREF_CYCLE_A");
    std::env::remove_var("RUBASH_NAMEREF_CYCLE_B");
    let input = format!(
        "declare -n RUBASH_NAMEREF_CYCLE_A=RUBASH_NAMEREF_CYCLE_B; \
         declare -n RUBASH_NAMEREF_CYCLE_B=RUBASH_NAMEREF_CYCLE_A; \
         printf 'braced=<%s>\\n' \"${{RUBASH_NAMEREF_CYCLE_A-unset}}\" > {shell_output_path}; \
         printf 'plain=<%s>\\n' \"$RUBASH_NAMEREF_CYCLE_A\" >> {shell_output_path}; \
         RUBASH_NAMEREF_CYCLE_A=value; \
         echo assign:$? >> {shell_output_path}; \
         declare -p RUBASH_NAMEREF_CYCLE_A RUBASH_NAMEREF_CYCLE_B >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "braced=<unset>\nplain=<>\nassign:1\ndeclare -n RUBASH_NAMEREF_CYCLE_A=\"RUBASH_NAMEREF_CYCLE_B\"\ndeclare -n RUBASH_NAMEREF_CYCLE_B=\"RUBASH_NAMEREF_CYCLE_A\"\n"
    );
    std::env::remove_var("RUBASH_NAMEREF_CYCLE_A");
    std::env::remove_var("RUBASH_NAMEREF_CYCLE_B");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_comma_sequences_evaluate_in_order() {
    let output_path = "target/rubash-arithmetic-command-comma-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=0; (( n = 1, n += 2, n )); echo $? $n > {output_path}; (( n++, n++, n - 5 )); echo $? $n >> {output_path}; (( n = 0, n )); echo $? $n >> {output_path}; (( (1, 2) )); echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0 3\n1 5\n1 0\n0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_comparison_operators() {
    let output_path = "target/rubash-arithmetic-command-comparison-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=3; (( n > 2 )); echo $? > {output_path}; (( n < 2 )); echo $? >> {output_path}; (( n >= 3 )); echo $? >> {output_path}; (( n <= 2 )); echo $? >> {output_path}; (( n == 3 )); echo $? >> {output_path}; (( n != 3 )); echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n1\n0\n1\n0\n1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_logical_operators() {
    let output_path = "target/rubash-arithmetic-command-logical-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=3; (( n > 2 && n < 4 )); echo $? > {output_path}; (( n > 2 && n < 3 )); echo $? >> {output_path}; (( n > 5 || n == 3 )); echo $? >> {output_path}; (( n > 5 || n < 0 )); echo $? >> {output_path}; (( !0 )); echo $? >> {output_path}; (( !n )); echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n1\n0\n1\n0\n1\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_accepts_dollar_variables() {
    let output_path = "target/rubash-arithmetic-command-dollar-vars-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "now1=100; now2=100; offset=1; \
         (( $now1 - $offset <= $now2 && ${{now2}} <= $now1 + $offset )); echo $? > {output_path}; \
         echo $(( $now1 + ${{offset}} )) >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n101\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_logical_operators_short_circuit() {
    let output_path = "target/rubash-arithmetic-command-short-circuit-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "(( 1 || 1 / 0 )); echo $? > {output_path}; (( 0 && 1 / 0 )); echo $? >> {output_path}; (( 0 && 1 / 0 || 4 )); echo $? >> {output_path}; (( 1 || (1 / 0), 0 )); echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n1\n0\n1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_command_bitwise_and_shift_operators() {
    let output_path = "target/rubash-arithmetic-command-bitwise-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=6; (( n & 2 )); echo $? > {output_path}; (( n & 1 )); echo $? >> {output_path}; (( 1 << 3 | 2 )); echo $? >> {output_path}; (( 14 >> 2 )); echo $? >> {output_path}; [[ 5^3 -eq 6 ]]; echo $? >> {output_path}; (( ~0 + 1 )); echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0\n1\n0\n0\n0\n1\n"
    );
    let _ = fs::remove_file(output_path);
}
