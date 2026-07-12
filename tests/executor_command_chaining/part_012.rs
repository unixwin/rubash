use super::super::*;
use std::fs;

#[test]
fn test_mapfile_without_t_preserves_record_newlines() {
    let input = "mapfile arr <<< $'alpha\\nbeta'";
    let tokens = tokenize(input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        executor.get_env("arr"),
        Some("\x1d([0]=$'alpha\\n' [1]=$'beta\\n')")
    );
}
#[test]
fn test_mapfile_t_preserves_trailing_empty_line() {
    let output_path = "target/rubash-mapfile-trailing-empty-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("mapfile -t arr <<< $'alpha\\n'; echo ${{#arr[@]}} ${{#arr[1]}} > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 0\n");
    let _ = fs::remove_file(output_path);
}
#[test]
fn test_readarray_t_reads_here_string_into_array() {
    let output_path = "target/rubash-readarray-t-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "readarray -t arr <<< $'alpha\\nbeta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 alpha beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_mapfile_n_limits_read_lines() {
    let output_path = "target/rubash-mapfile-n-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "mapfile -n 2 -t arr <<< $'alpha\\nbeta\\ngamma'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 alpha beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_mapfile_n_zero_reads_all_lines() {
    let output_path = "target/rubash-mapfile-n-zero-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "mapfile -n 0 -t arr <<< $'alpha\\nbeta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 alpha beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_mapfile_callback_runs_at_quantum() {
    let output_path = "target/rubash-mapfile-callback-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "cb() {{ echo \"$1:$2\" >> {output_path}; }}; mapfile -C cb -c 2 -t arr <<< $'alpha\\nbeta\\ngamma'; echo ${{#arr[@]}} ${{arr[@]}} >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "1:beta\n3 alpha beta gamma\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_readarray_compact_n_limits_read_lines() {
    let output_path = "target/rubash-readarray-compact-n-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "readarray -n1 -t arr <<< $'alpha\\nbeta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1 alpha\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_mapfile_s_skips_initial_lines() {
    let output_path = "target/rubash-mapfile-s-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "mapfile -s 1 -t arr <<< $'alpha\\nbeta\\ngamma'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 beta gamma\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_readarray_compact_s_combines_with_n() {
    let output_path = "target/rubash-readarray-compact-s-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "readarray -s1 -n1 -t arr <<< $'alpha\\nbeta\\ngamma'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1 beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_mapfile_o_sets_origin_index() {
    let input = "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta'";
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        executor.get_env("arr"),
        Some("\x1d([2]=\"alpha\" [3]=\"beta\")")
    );
}

#[test]
fn test_readarray_compact_o_preserves_existing_elements() {
    let input = "arr=(zero one two); readarray -O2 -n1 -t arr <<< $'new\\nmore'";
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        executor.get_env("arr"),
        Some("\x1d([0]=\"zero\" [1]=\"one\" [2]=\"new\")")
    );
}

#[test]
fn test_mapfile_d_uses_custom_delimiter() {
    let input = "mapfile -d : -t arr <<< 'alpha:beta:gamma'";
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        executor.get_env("arr"),
        Some("\x1d([0]=\"alpha\" [1]=\"beta\" [2]=$'gamma\\n')")
    );
}

#[test]
fn test_readarray_compact_d_keeps_delimiter_without_t() {
    let input = "readarray -d: arr <<< 'alpha:beta'";
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        executor.get_env("arr"),
        Some("\x1d([0]=\"alpha:\" [1]=$'beta\\n')")
    );
}

#[test]
fn test_mapfile_u_reads_numbered_fd_here_string() {
    let output_path = "target/rubash-mapfile-u-fd-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "mapfile -u 3 -t arr 3<<<$'alpha\\nbeta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2 alpha beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_readarray_compact_u_reads_numbered_fd_file() {
    let input_path = "target/rubash-readarray-u-fd-input.txt";
    fs::write(input_path, "alpha\nbeta\n").unwrap();
    let input = format!("readarray -u3 -t arr 3<{input_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        executor.get_env("arr"),
        Some("\x1d([0]=\"alpha\" [1]=\"beta\")")
    );
    let _ = fs::remove_file(input_path);
}

#[test]
fn test_array_at_indices_expand() {
    let output_path = "target/rubash-array-at-indices-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "unset arr; mapfile -O 2 -t arr <<< $'alpha\\nbeta'; echo ${{!arr[@]}} > {output_path}"
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
