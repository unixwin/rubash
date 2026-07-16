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
fn test_quoted_array_length_counts_elements() {
    let output_path = "target/rubash-quoted-array-length-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "arr=([2]=two [5]=five); declare -A assoc=([one]=1 [two]=2); \
         printf '%s:%s\\n' \"${{#arr[@]}}\" \"${{#assoc[@]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "2:2\n");
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

#[test]
fn test_mapfile_u_rejects_invalid_fd_specifications() {
    let output_path = "target/rubash-mapfile-u-invalid-fd-status.txt";
    let error_path = "target/rubash-mapfile-u-invalid-fd-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "mapfile -u x arr <<< abc 2> {error_path}; echo word:$? > {output_path}; \
         readarray -u-1 arr <<< abc 2>> {error_path}; echo compact_negative:$? >> {output_path}; \
         mapfile -u2147483648 arr <<< abc 2>> {error_path}; echo too_large:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "word:1\ncompact_negative:1\ntoo_large:1\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("mapfile: x: invalid file descriptor specification"));
    assert!(error.contains("readarray: -1: invalid file descriptor specification"));
    assert!(error.contains("mapfile: 2147483648: invalid file descriptor specification"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_mapfile_u_reports_bad_fd_for_unopened_or_closed_fd() {
    let output_path = "target/rubash-mapfile-u-bad-fd-status.txt";
    let error_path = "target/rubash-mapfile-u-bad-fd-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "mapfile -u3 arr 2> {error_path}; echo unopened:$? > {output_path}; \
         readarray -u 3 arr 3<&- 2>> {error_path}; echo closed:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "unopened:1\nclosed:1\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("mapfile: 3: invalid file descriptor: Bad file descriptor"));
    assert!(error.contains("readarray: 3: invalid file descriptor: Bad file descriptor"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_mapfile_rejects_invalid_array_name() {
    let output_path = "target/rubash-mapfile-invalid-array-status.txt";
    let error_path = "target/rubash-mapfile-invalid-array-error.txt";
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
    let input = format!(
        "mapfile 1bad <<< abc 2> {error_path}; echo mapfile:$? > {output_path}; \
         readarray 2bad <<< abc 2>> {error_path}; echo readarray:$? >> {output_path}; \
         mapfile -O 0 3bad <<< abc 2>> {error_path}; echo option:$? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "mapfile:1\nreadarray:1\noption:1\n"
    );
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("mapfile: `1bad': not a valid identifier"));
    assert!(error.contains("readarray: `2bad': not a valid identifier"));
    assert!(error.contains("mapfile: `3bad': not a valid identifier"));
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_mapfile_ignores_extra_array_names_and_accepts_double_dash() {
    let output_path = "target/rubash-mapfile-extra-array-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "mapfile ok extra <<< abc; \
         mapfile -- dashed <<< def; \
         printf '%s:%s:%s:%s' \"${{#ok[@]}}\" \"${{#extra[@]}}\" \"${{#dashed[@]}}\" \"${{dashed[0]}}\" > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1:0:1:def\n");
    let _ = fs::remove_file(output_path);
}
