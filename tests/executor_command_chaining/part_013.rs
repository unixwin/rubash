use super::super::*;
use std::fs;

#[test]
fn test_array_star_indices_expand() {
    let output_path = "target/rubash-array-star-indices-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta'; echo ${{!arr[*]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 3\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_array_at_indices_expand_as_loop_words() {
    let output_path = "target/rubash-quoted-array-indices-loop-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(apple banana cherry); for i in \"${{!arr[@]}}\"; do echo \"[$i]=${{arr[$i]}}\" >> {output_path}; done"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "[0]=apple\n[1]=banana\n[2]=cherry\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_quoted_assoc_at_indices_expand_as_loop_words() {
    let output_path = "target/rubash-quoted-assoc-indices-loop-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -A loop_assoc_indices=([red]=apple [blue]=berry); for key in \"${{!loop_assoc_indices[@]}}\"; do echo \"$key=${{loop_assoc_indices[$key]}}\" >> {output_path}; done"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "red=apple\nblue=berry\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_numeric_subscript_expands_element() {
    let output_path = "target/rubash-array-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("arr=(zero one two); echo ${{arr[1]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "one\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_negative_subscript_expands_from_end() {
    let output_path = "target/rubash-array-negative-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one two); printf '%s:%s:%s\\n' \"${{arr[-1]}}\" \"${{arr[-2]}}\" \"${{#arr[-1]}}\" > {output_path}; sparse=([2]=two [5]=five); printf '%s:%s\\n' \"${{sparse[-1]}}\" \"${{sparse[-4]}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "two:one:3\nfive:two\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_negative_subscript_assignment_updates_from_end() {
    let output_path = "target/rubash-array-negative-subscript-assign-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one two); arr[-1]=TWO; arr[-2]=ONE; printf '%s %s %s\\n' \"${{arr[0]}}\" \"${{arr[1]}}\" \"${{arr[2]}}\" > {output_path}; arr[-1]+=!; echo \"${{arr[2]}}\" >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "zero ONE TWO\nTWO!\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_adjacent_braced_parameter_expansions_stay_in_one_word() {
    let output_path = "target/rubash-adjacent-braced-param-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=(zero one two); left=alpha; right=beta; echo ${{arr[1]}}:${{arr[2]}} > {output_path}; echo ${{left}}:${{right}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "one:two\nalpha:beta\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_sparse_array_numeric_subscript_expands_element() {
    let output_path = "target/rubash-sparse-array-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta'; echo ${{arr[2]}} ${{arr[3]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_array_numeric_subscript_length_expands() {
    let output_path = "target/rubash-array-subscript-length-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta'; echo ${{#arr[2]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "5\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_assoc_array_subscript_expands_element() {
    let output_path = "target/rubash-assoc-subscript-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("declare -A assoc; assoc[one]=alpha; assoc[two]=beta; echo ${{assoc[one]}} ${{assoc[two]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_assoc_array_subscript_length_expands() {
    let output_path = "target/rubash-assoc-subscript-length-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("declare -A assoc; assoc[one]=alpha; echo ${{#assoc[one]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "5\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_assoc_array_indices_expand_keys() {
    let output_path = "target/rubash-assoc-indices-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "declare -A assoc; assoc[one]=alpha; assoc[two]=beta; echo ${{!assoc[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "one two\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_cmds_reflects_hash_table() {
    let output_path = "target/rubash-bash-cmds-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "hash -r; \
         hash -p /usr/sbin/foo foo; \
         BASH_CMDS[bar]=/usr/bin/bar; \
         printf '%s\\n' \"${{!BASH_CMDS[@]}}\" \"${{BASH_CMDS[@]}}\" \"${{BASH_CMDS[foo]}}\" > {output_path}; \
         hash -t bar >> {output_path}; \
         unset 'BASH_CMDS[foo]'; hash -t foo 2>/dev/null; echo $? >> {output_path}; \
         declare -p BASH_CMDS >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "bar foo\n/usr/bin/bar /usr/sbin/foo\n/usr/sbin/foo\n/usr/bin/bar\n1\ndeclare -A BASH_CMDS=([bar]=\"/usr/bin/bar\" )\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_bash_aliases_reflects_alias_table() {
    let output_path = "target/rubash-bash-aliases-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "alias foo=/usr/sbin/foo; \
         BASH_ALIASES[bar]=/usr/bin/bar; \
         printf '%s\\n' \"${{!BASH_ALIASES[@]}}\" \"${{BASH_ALIASES[@]}}\" \"${{BASH_ALIASES[foo]}}\" > {output_path}; \
         unset 'BASH_ALIASES[foo]'; \
         alias foo 2>/dev/null; echo $? >> {output_path}; \
         declare -p BASH_ALIASES >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "bar foo\n/usr/bin/bar /usr/sbin/foo\n/usr/sbin/foo\n1\ndeclare -A BASH_ALIASES=([bar]=\"/usr/bin/bar\" )\n"
    );
    let _ = fs::remove_file(output_path);
}
