use super::*;

fn run(args: &[&str]) -> (i32, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let status = execute_with_io(args.iter().copied(), &mut stdout, &mut stderr).unwrap();

    (
        status,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn no_operands_succeeds() {
    assert_eq!(run(&[]), (EXECUTION_SUCCESS, String::new(), String::new()));
}

#[test]
fn reports_builtin_type() {
    assert_eq!(
        run(&["-t", "echo"]),
        (EXECUTION_SUCCESS, "builtin\n".to_string(), String::new())
    );
}

#[test]
fn reports_extended_builtins() {
    assert_eq!(
        run(&["-t", "read", "mapfile", "declare", "alias"]),
        (
            EXECUTION_SUCCESS,
            "builtin\nbuiltin\nbuiltin\nbuiltin\n".to_string(),
            String::new()
        )
    );
}

#[test]
fn rejects_invalid_options() {
    let (status, stdout, stderr) = run(&["-z"]);

    assert_eq!(status, EX_USAGE);
    assert!(stdout.is_empty());
    assert!(stderr.contains("invalid option"));
}
