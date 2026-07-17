use super::*;
use std::collections::HashMap;
use std::fs;
use std::io::IsTerminal;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
fn make_fifo(path: &str) {
    let path = std::ffi::CString::new(path).unwrap();
    let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
    assert_eq!(result, 0);
}

fn run(args: &[&str], bracket: bool) -> (i32, String) {
    let env_vars = HashMap::new();
    run_with_env(args, bracket, &env_vars)
}

fn run_with_env(args: &[&str], bracket: bool, env_vars: &HashMap<String, String>) -> (i32, String) {
    let mut stderr = Vec::new();
    let status = execute_with_stderr(args.iter().copied(), bracket, env_vars, &mut stderr).unwrap();
    (status, String::from_utf8(stderr).unwrap())
}

#[test]
fn empty_expression_is_false() {
    assert_eq!(run(&[], false).0, EXECUTION_FAILURE);
}

#[test]
fn single_non_empty_string_is_true() {
    assert_eq!(run(&["hello"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&[""], false).0, EXECUTION_FAILURE);
}

#[test]
fn supports_string_and_numeric_binary_operators() {
    assert_eq!(run(&["a", "=", "a"], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["2", "-lt", "3"], false).0, EXECUTION_SUCCESS);
}

#[test]
fn supports_not_and_logical_operators() {
    assert_eq!(run(&["!", ""], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["x", "-a", ""], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["x", "-o", ""], false).0, EXECUTION_SUCCESS);
}

#[test]
fn supports_shell_option_unary_operator() {
    let mut env_vars = HashMap::new();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    crate::builtins::set::set_with_io(["-o", "errexit"], &mut env_vars, &mut stdout, &mut stderr)
        .unwrap();

    assert_eq!(
        run_with_env(&["-o", "errexit"], false, &env_vars).0,
        EXECUTION_SUCCESS
    );
    crate::builtins::set::set_with_io(["+o", "errexit"], &mut env_vars, &mut stdout, &mut stderr)
        .unwrap();
    assert_eq!(
        run_with_env(&["-o", "errexit"], false, &env_vars).0,
        EXECUTION_FAILURE
    );
    assert_eq!(
        run_with_env(&["-o", "no_such_option"], false, &env_vars).0,
        EXECUTION_FAILURE
    );
}

#[test]
fn leading_file_operator_is_unary_not_logical_and() {
    assert_eq!(run(&["-a", "Cargo.toml"], false).0, EXECUTION_SUCCESS);
}

#[test]
fn modified_since_read_unary_operator_checks_existing_file() {
    let path = "target/rubash-test-n-unary.txt";
    let missing = "target/rubash-test-n-unary-missing.txt";
    let _ = fs::create_dir_all("target");
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(missing);
    fs::write(path, "data").unwrap();

    assert_eq!(run(&["-N", path], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-N", missing], false).0, EXECUTION_FAILURE);

    let _ = fs::remove_file(path);
}

#[test]
fn ownership_unary_operators_check_existing_file() {
    let path = "target/rubash-test-owner-unary.txt";
    let missing = "target/rubash-test-owner-unary-missing.txt";
    let _ = fs::create_dir_all("target");
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(missing);
    fs::write(path, "data").unwrap();

    let expected = if cfg!(unix) {
        EXECUTION_SUCCESS
    } else {
        EXECUTION_FAILURE
    };
    assert_eq!(run(&["-O", path], false).0, expected);
    assert_eq!(run(&["-G", path], false).0, expected);
    assert_eq!(run(&["-O", missing], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["-G", missing], false).0, EXECUTION_FAILURE);

    let _ = fs::remove_file(path);
}

#[cfg(unix)]
#[test]
fn unix_file_unary_operators_check_file_types_and_mode_bits() {
    let fifo_path = "target/rubash-test-file-kind-fifo";
    let socket_path = "target/rubash-test-file-kind-socket";
    let mode_path = "target/rubash-test-file-kind-mode.txt";
    let sticky_dir = "target/rubash-test-file-kind-sticky-dir";
    let _ = fs::remove_file(fifo_path);
    let _ = fs::remove_file(socket_path);
    let _ = fs::remove_file(mode_path);
    let _ = fs::remove_dir_all(sticky_dir);

    make_fifo(fifo_path);
    let _socket = std::os::unix::net::UnixListener::bind(socket_path).unwrap();
    fs::write(mode_path, "data").unwrap();
    let mut permissions = fs::metadata(mode_path).unwrap().permissions();
    permissions.set_mode(0o7600);
    fs::set_permissions(mode_path, permissions).unwrap();
    fs::create_dir_all(sticky_dir).unwrap();
    let mut permissions = fs::metadata(sticky_dir).unwrap().permissions();
    permissions.set_mode(0o1700);
    fs::set_permissions(sticky_dir, permissions).unwrap();

    assert_eq!(run(&["-p", fifo_path], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-S", socket_path], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-u", mode_path], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-g", mode_path], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-k", sticky_dir], false).0, EXECUTION_SUCCESS);
    assert_eq!(run(&["-b", mode_path], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["-c", mode_path], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["-p", mode_path], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["-S", mode_path], false).0, EXECUTION_FAILURE);

    let _ = fs::remove_file(fifo_path);
    let _ = fs::remove_file(socket_path);
    let _ = fs::remove_file(mode_path);
    let _ = fs::remove_dir_all(sticky_dir);
}

#[cfg(not(unix))]
#[test]
fn unix_file_unary_operators_default_false_on_non_unix() {
    let path = "target/rubash-test-file-kind-non-unix.txt";
    let _ = fs::create_dir_all("target");
    let _ = fs::remove_file(path);
    fs::write(path, "data").unwrap();

    for op in ["-b", "-c", "-g", "-k", "-p", "-S", "-u"] {
        assert_eq!(run(&[op, path], false).0, EXECUTION_FAILURE);
    }

    let _ = fs::remove_file(path);
}

#[test]
fn terminal_unary_operator_checks_standard_fds() {
    let expected_stdin = if std::io::stdin().is_terminal() {
        EXECUTION_SUCCESS
    } else {
        EXECUTION_FAILURE
    };
    let expected_stdout = if std::io::stdout().is_terminal() {
        EXECUTION_SUCCESS
    } else {
        EXECUTION_FAILURE
    };
    let expected_stderr = if std::io::stderr().is_terminal() {
        EXECUTION_SUCCESS
    } else {
        EXECUTION_FAILURE
    };

    assert_eq!(run(&["-t", "0"], false).0, expected_stdin);
    assert_eq!(run(&["-t", "1"], false).0, expected_stdout);
    assert_eq!(run(&["-t", "2"], false).0, expected_stderr);
    assert_eq!(run(&["-t", "9999"], false).0, EXECUTION_FAILURE);
    assert_eq!(run(&["-t", "nope"], false).0, EXECUTION_FAILURE);
}

#[test]
fn supports_parenthesized_logical_expressions() {
    assert_eq!(
        run(&["(", "", "-o", "x", ")", "-a", ""], false).0,
        EXECUTION_FAILURE
    );
    assert_eq!(
        run(&["\\(", "", "-o", "x", "\\)", "-a", "x"], false).0,
        EXECUTION_SUCCESS
    );
}

#[test]
fn bracket_requires_closing_bracket() {
    let (status, stderr) = run(&["x"], true);

    assert_eq!(status, EX_BADUSAGE);
    assert!(stderr.contains("missing `]'"));
}
