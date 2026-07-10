use super::super::*;
use std::fs;

#[test]
fn test_arithmetic_array_subscripts_read_and_update() {
    let output_path = "target/rubash-arithmetic-array-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(4 7); i=1; echo $((arr[i]+1)) > {output_path}; (( arr[i] += 2 )); echo ${{arr[1]}} $((arr[i])) >> {output_path}; (( arr[0]++ )); echo ${{arr[0]}} >> {output_path}; echo $((arr[2])) >> {output_path}; arr[2]=5; echo $((arr[2])) >> {output_path}; i=0; echo $((arr[i++])) $i >> {output_path}; (( arr[-1] += 4 )); echo ${{arr[2]}} $((arr[-1])) >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "8\n9 9\n5\n0\n5\n5 1\n9 9\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_assoc_subscripts_read_and_update() {
    let output_path = "target/rubash-arithmetic-assoc-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -A assoc; key=one; assoc[one]=7; echo $((assoc[one]+1)) > {output_path}; echo $((assoc[key]+1)) >> {output_path}; (( assoc[key] += 2 )); echo ${{assoc[one]}} ${{assoc[key]}} $((assoc[key])) >> {output_path}; (( assoc[two]++ )); echo ${{assoc[two]}} >> {output_path}; echo $((assoc[missing])) >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "8\n1\n7 2 2\n1\n0\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_assoc_subscripts_expand_parameter_keys() {
    let output_path = "target/rubash-arithmetic-assoc-expanded-key-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -A arith_assoc_expanded_key=([\"two words\"]=7 [key]=3); arith_assoc_key='two words'; echo $((arith_assoc_expanded_key[key]+1)) > {output_path}; echo $((arith_assoc_expanded_key[$arith_assoc_key]+1)) >> {output_path}; echo $((arith_assoc_expanded_key[\"$arith_assoc_key\"]+1)) >> {output_path}; (( arith_assoc_expanded_key[$arith_assoc_key] += 2 )); echo ${{arith_assoc_expanded_key[\"two words\"]}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "4\n8\n8\n9\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_arithmetic_lvalues_resolve_nameref_targets() {
    let output_path = "target/rubash-arithmetic-nameref-lvalue-output.txt";
    let _ = fs::remove_file(output_path);
    for name in [
        "RUBASH_ARITH_NAMEREF_SCALAR",
        "RUBASH_ARITH_NAMEREF_SCALAR_REF",
        "RUBASH_ARITH_NAMEREF_ARRAY",
        "RUBASH_ARITH_NAMEREF_ARRAY_REF",
        "RUBASH_ARITH_NAMEREF_ASSOC",
        "RUBASH_ARITH_NAMEREF_ASSOC_REF",
        "RUBASH_ARITH_NAMEREF_KEY",
    ] {
        std::env::remove_var(name);
    }
    let input = format!(
        "RUBASH_ARITH_NAMEREF_SCALAR=1; \
         declare -n RUBASH_ARITH_NAMEREF_SCALAR_REF=RUBASH_ARITH_NAMEREF_SCALAR; \
         (( RUBASH_ARITH_NAMEREF_SCALAR_REF += 2 )); (( ++RUBASH_ARITH_NAMEREF_SCALAR_REF )); \
         echo scalar:${{RUBASH_ARITH_NAMEREF_SCALAR}}:${{RUBASH_ARITH_NAMEREF_SCALAR_REF}} > {output_path}; \
         RUBASH_ARITH_NAMEREF_ARRAY=(10 20); \
         declare -n RUBASH_ARITH_NAMEREF_ARRAY_REF=RUBASH_ARITH_NAMEREF_ARRAY; \
         (( RUBASH_ARITH_NAMEREF_ARRAY_REF[1] += 5 )); \
         echo indexed:${{RUBASH_ARITH_NAMEREF_ARRAY[1]}}:${{RUBASH_ARITH_NAMEREF_ARRAY_REF[1]}} >> {output_path}; \
         declare -A RUBASH_ARITH_NAMEREF_ASSOC=([k]=7); RUBASH_ARITH_NAMEREF_KEY=k; \
         declare -n RUBASH_ARITH_NAMEREF_ASSOC_REF=RUBASH_ARITH_NAMEREF_ASSOC; \
         (( RUBASH_ARITH_NAMEREF_ASSOC_REF[$RUBASH_ARITH_NAMEREF_KEY] += 2 )); \
         echo assoc:${{RUBASH_ARITH_NAMEREF_ASSOC[k]}}:${{RUBASH_ARITH_NAMEREF_ASSOC_REF[k]}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "scalar:4:4\nindexed:25:25\nassoc:9:9\n"
    );
    let _ = fs::remove_file(output_path);
    for name in [
        "RUBASH_ARITH_NAMEREF_SCALAR",
        "RUBASH_ARITH_NAMEREF_SCALAR_REF",
        "RUBASH_ARITH_NAMEREF_ARRAY",
        "RUBASH_ARITH_NAMEREF_ARRAY_REF",
        "RUBASH_ARITH_NAMEREF_ASSOC",
        "RUBASH_ARITH_NAMEREF_ASSOC_REF",
        "RUBASH_ARITH_NAMEREF_KEY",
    ] {
        std::env::remove_var(name);
    }
}

#[test]
fn test_grouped_arithmetic_assignments_have_side_effects() {
    let output_path = "target/rubash-arithmetic-grouped-assign-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "n=0; (( (n = 3) )); echo $? $n > {output_path}; (( ((m = 0)) )); echo $? $m >> {output_path}; (( (1 ? 4 : 1 / 0) )); echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0 3\n1 0\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_let_builtin_evaluates_arithmetic_expressions() {
    let output_path = "target/rubash-let-arithmetic-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "let n=1 n+=2 n; echo $? $n > {output_path}; let n=0; echo $? $n >> {output_path}; let n=2 n**=3 n-8; echo $? $n >> {output_path}; let n/=0; echo $? $n >> {output_path}; let; echo $? >> {output_path}; a=() b=(); let a=(5 + 3) b=(4 + 7); echo $? $a $b >> {output_path}; let a=(4*3)/2; echo $? $a >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0 3\n1 0\n1 8\n1 8\n1\n0 8 11\n0 6\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_integer_variables_evaluate_assignments_as_arithmetic() {
    let output_path = "target/rubash-integer-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -i n=2+3*4; echo $n > {output_path}; n=2**3; echo $n >> {output_path}; n+=2*5; echo $n >> {output_path}; declare -p n >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "14\n8\n18\ndeclare -i n=\"18\"\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_integer_compound_assignment_evaluates_as_scalar_arithmetic() {
    let output_path = "target/rubash-integer-compound-assignment-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "typeset -i a b; a=(5+3) b=(4+7); echo $a $b > {output_path}; declare -p a b >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "8 11\ndeclare -i a=\"8\"\ndeclare -i b=\"11\"\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_case_conversion_variables_transform_assignments() {
    let output_path = "target/rubash-case-conversion-vars-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -u U=abc; echo $U > {output_path}; U=def; echo $U >> {output_path}; \
         declare -l L=ABC; echo $L >> {output_path}; L=XYZ; echo $L >> {output_path}; \
         declare -p U L >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "ABC\nDEF\nabc\nxyz\ndeclare -u U=\"DEF\"\ndeclare -l L=\"xyz\"\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_declare_plus_attributes_clear_variable_flags() {
    let output_path = "target/rubash-declare-plus-attrs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -u U=abc; declare +u U; U=ghi; echo $U > {output_path}; declare -p U >> {output_path}; \
         declare -l L=ABC; declare +l L; L=XYZ; echo $L >> {output_path}; declare -p L >> {output_path}; \
         declare -i n=2+3; declare +i n; n=2+3; echo $n >> {output_path}; declare -p n >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "ghi\ndeclare -- U=\"ghi\"\nXYZ\ndeclare -- L=\"XYZ\"\n2+3\ndeclare -- n=\"2+3\"\n"
    );
    let _ = fs::remove_file(output_path);
}
