use super::super::*;
use std::fs;

#[test]
fn test_builtin_bracket_invokes_test_builtin() {
    let output_path = "target/rubash-builtin-bracket-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin [ 3 -eq 4 ]; echo $? > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_test_redirect_truncates_output_file() {
    let output_path = "target/rubash-builtin-test-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("builtin test 3 -eq 3 > {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_let_invokes_let_builtin() {
    let output_path = "target/rubash-builtin-let-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin let n=1+2 n; echo $? $n > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0 3\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_read_uses_here_string() {
    let output_path = "target/rubash-builtin-read-output.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("builtin read left right <<< 'alpha beta'; echo $left/$right > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "alpha/beta\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_mapfile_uses_here_string() {
    let output_path = "target/rubash-builtin-mapfile-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "builtin mapfile -t arr <<< $'alpha\\nbeta'; echo ${{#arr[@]}} ${{arr[@]}} > {output_path}"
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
fn test_builtin_umask_redirects_output() {
    let output_path = "target/rubash-builtin-umask-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin umask 077; builtin umask > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "0077\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_trap_redirects_output() {
    let output_path = "target/rubash-builtin-trap-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin trap 'echo bye' EXIT; builtin trap -p EXIT > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "trap -- 'echo bye' EXIT\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_source_updates_current_shell() {
    let script_path = "target/rubash-builtin-source-script.sh";
    let output_path = "target/rubash-builtin-source-output.txt";
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    fs::write(script_path, "RUBASH_BUILTIN_SOURCE=ok\n").unwrap();
    let input =
        format!("builtin source {script_path}; echo $RUBASH_BUILTIN_SOURCE > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_dot_updates_current_shell() {
    let script_path = "target/rubash-builtin-dot-script.sh";
    let output_path = "target/rubash-builtin-dot-output.txt";
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    fs::write(script_path, "RUBASH_BUILTIN_DOT=ok\n").unwrap();
    let input = format!("builtin . {script_path}; echo $RUBASH_BUILTIN_DOT > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_source_searches_path_before_current_directory() {
    let local_script = "rubash-sourcepath-temp.sh";
    let bin_dir = "target/rubash-sourcepath-bin";
    let path_script = format!("{bin_dir}/{local_script}");
    let output_path = "target/rubash-sourcepath-output.txt";
    let _ = fs::remove_file(local_script);
    let _ = fs::remove_file(&path_script);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    fs::write(local_script, "RUBASH_SOURCEPATH=local\n").unwrap();
    fs::write(&path_script, "RUBASH_SOURCEPATH=path\n").unwrap();
    let input =
        format!("PATH={bin_dir}; source {local_script}; echo $RUBASH_SOURCEPATH > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "path\n");
    let _ = fs::remove_file(local_script);
    let _ = fs::remove_file(path_script);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir(bin_dir);
}

#[test]
fn test_sourcepath_disabled_uses_current_directory() {
    let local_script = "rubash-sourcepath-disabled-temp.sh";
    let bin_dir = "target/rubash-sourcepath-disabled-bin";
    let path_script = format!("{bin_dir}/{local_script}");
    let output_path = "target/rubash-sourcepath-disabled-output.txt";
    let _ = fs::remove_file(local_script);
    let _ = fs::remove_file(&path_script);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    fs::write(local_script, "RUBASH_SOURCEPATH_DISABLED=local\n").unwrap();
    fs::write(&path_script, "RUBASH_SOURCEPATH_DISABLED=path\n").unwrap();
    let input = format!(
        "PATH={bin_dir}; shopt -u sourcepath; source {local_script}; shopt -s sourcepath; echo $RUBASH_SOURCEPATH_DISABLED > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "local\n");
    let _ = fs::remove_file(local_script);
    let _ = fs::remove_file(path_script);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir(bin_dir);
}

#[test]
fn test_source_p_empty_path_uses_current_directory() {
    let script_path = "rubash-source-p-empty-temp.sh";
    let output_path = "target/rubash-source-p-empty-output.txt";
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    fs::write(script_path, "RUBASH_SOURCE_P_EMPTY=ok\n").unwrap();
    let input = format!(
        "PATH=/no/such/path; source -p '' {script_path}; echo $RUBASH_SOURCE_P_EMPTY > {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "ok\n");
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
}
