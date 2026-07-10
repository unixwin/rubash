use super::super::*;
use std::fs;

#[test]
fn test_builtin_cd_invokes_cd_builtin() {
    let original_dir = std::env::current_dir().unwrap();
    let original_pwd = std::env::var("PWD").ok();
    let original_oldpwd = std::env::var("OLDPWD").ok();
    let root = original_dir.join("target/rubash-builtin-cd");
    let dest_dir = root.join("dest");
    let output_path = root.join("output.txt");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&dest_dir).unwrap();

    let dest_display = shell_test_path(&dest_dir);
    let output_display = output_path.to_string_lossy().replace('\\', "/");
    let input = format!(
        "function cd {{ echo function-cd; }}; builtin cd {dest_display}; pwd > {output_display}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);
    let _ = std::env::set_current_dir(&original_dir);
    match original_pwd {
        Some(value) => std::env::set_var("PWD", value),
        None => std::env::remove_var("PWD"),
    }
    match original_oldpwd {
        Some(value) => std::env::set_var("OLDPWD", value),
        None => std::env::remove_var("OLDPWD"),
    }

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        format!("{dest_display}\n")
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_builtin_set_updates_shell_options() {
    let output_path = "target/rubash-builtin-set-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "builtin set -u; echo $- > {output_path}; [[ -o nounset ]]; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let lines: Vec<String> = fs::read_to_string(output_path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert!(lines[0].contains('u'));
    assert_eq!(lines[1], "0");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_set_replaces_positional_parameters() {
    let output_path = "target/rubash-builtin-set-positionals-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin set -- alpha beta; echo $# $1 $2 > {output_path}");
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
fn test_cd_redirects_stderr() {
    let error_path = "target/rubash-cd-stderr-output.txt";
    let status_path = "target/rubash-cd-stderr-status.txt";
    let missing_dir = "target/rubash-cd-no-such-dir";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);

    let input = format!("cd {missing_dir} 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("rubash: cd:"));
    assert!(error.contains("rubash-cd-no-such-dir"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_cd_appends_stderr() {
    let error_path = "target/rubash-cd-stderr-append-output.txt";
    let missing_dir = "target/rubash-cd-no-such-dir";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();

    let input = format!("cd {missing_dir} 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("rubash: cd:"));
    assert!(error.contains("rubash-cd-no-such-dir"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_umask_redirects_output() {
    let output_path = "target/rubash-umask-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("umask 077; umask > {output_path}");
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
fn test_umask_appends_output() {
    let output_path = "target/rubash-umask-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("umask 077; umask >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "before\n0077\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_umask_symbolic_modes_update_mask() {
    let output_path = "target/rubash-umask-symbolic-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "umask 022; umask u=rwx,g=rx,o=; umask > {output_path}; \
         umask -S >> {output_path}; umask a+r; umask >> {output_path}; \
         umask -S >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "0027\nu=rwx,g=rx,o=\n0023\nu=rwx,g=rx,o=r\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_umask_symbolic_modes_copy_permissions() {
    let output_path = "target/rubash-umask-symbolic-copy-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "umask 022; umask g+u,o+rwx-u; umask -S > {output_path}; \
         umask 022; umask o=u; umask -p -S >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "u=rwx,g=rwx,o=\numask -S u=rwx,g=rx,o=rwx\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_umask_redirects_stderr() {
    let error_path = "target/rubash-umask-stderr-output.txt";
    let status_path = "target/rubash-umask-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("umask -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("rubash: umask: -Z: invalid option"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_umask_appends_stderr() {
    let error_path = "target/rubash-umask-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("umask -Z 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("rubash: umask: -Z: invalid option"));
    let _ = fs::remove_file(error_path);
}
