use super::super::*;
use std::fs;

#[test]
fn test_parameter_replacement_matches_literal_backslashes() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-parameter-backslash-replacement.txt";
    let _ = fs::remove_file(output_path);
    let input =
        format!("q=\"C:\\\\work\\\\dir\\\\file.txt\"; echo \"${{q//\\\\\\\\//}}\" > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    assert!(executor.execute_ast(&ast).is_ok());
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "C:/work/dir/file.txt\n"
    );
    let _ = fs::remove_file(output_path);
}

/// rubash#2932: `times` prints real getrusage values now, so the
/// redirect tests below pin the LINE SHAPE (print_timeval's
/// `{mins}m{secs}.{millis:03}s`, two fields per line, two lines)
/// instead of the old stub's byte-exact zeros — their subject is the
/// redirect plumbing, not the values.
fn assert_times_lines(text: &str) {
    fn valid_timeval(field: &str) -> bool {
        let Some((mins, rest)) = field.split_once('m') else {
            return false;
        };
        let Some((secs, frac)) = rest.split_once('.') else {
            return false;
        };
        let Some(frac) = frac.strip_suffix('s') else {
            return false;
        };
        !mins.is_empty()
            && mins.bytes().all(|b| b.is_ascii_digit())
            && !secs.is_empty()
            && secs.bytes().all(|b| b.is_ascii_digit())
            && frac.len() == 3
            && frac.bytes().all(|b| b.is_ascii_digit())
    }

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "text: {text}");
    for line in lines {
        let fields: Vec<&str> = line.split(' ').collect();
        assert_eq!(fields.len(), 2, "line: {line}");
        for field in fields {
            assert!(valid_timeval(field), "field: {field}");
        }
    }
}

#[test]
fn test_times_redirects_output() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-times-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("times > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_times_lines(&fs::read_to_string(output_path).unwrap());
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_times_appends_output() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-times-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("times >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let content = fs::read_to_string(output_path).unwrap();
    let rest = content
        .strip_prefix("before\n")
        .expect("append keeps prefix");
    assert_times_lines(rest);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_times_ignores_non_option_arguments() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-times-extra-args-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("times ignored > {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let content = fs::read_to_string(output_path).unwrap();
    let head = content.strip_suffix("0\n").expect("status line follows");
    assert_times_lines(head);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_times_redirects_stderr() {
    let _cwd_guard = CwdGuard::take();
    let error_path = "target/rubash-times-stderr-output.txt";
    let status_path = "target/rubash-times-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("times -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    // rubash#3357: the error-name prefix is derived through the name-source
    // chain, not the interactive bare-name form. This embedded executor
    // carries no name (no argv[0]), so the chain falls back to `bash`
    // (error.c:90 get_name_for_error) and the non-interactive form adds
    // the `line 1: ` segment (builtins/common.c:83 builtin_error_prolog).
    assert!(error.contains("bash: line 1: times: -x: invalid option"));
    assert!(error.contains("times: usage: times"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_times_appends_stderr() {
    let _cwd_guard = CwdGuard::take();
    let error_path = "target/rubash-times-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("times -x 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    // rubash#3357: derived name prefix, same as test_times_redirects_stderr
    // above — no-name carrier falls back to `bash`, non-interactive adds
    // `line 1: ` (error.c:90 get_name_for_error;
    // builtins/common.c:83 builtin_error_prolog).
    assert!(error.contains("bash: line 1: times: -x: invalid option"));
    assert!(error.contains("times: usage: times"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_help_redirects_output() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-help-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("help -s help > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "help: help [-dms] [pattern ...]\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_help_appends_output() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-help-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("help -s help >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\nhelp: help [-dms] [pattern ...]\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_help_redirects_stderr() {
    let _cwd_guard = CwdGuard::take();
    let error_path = "target/rubash-help-stderr-output.txt";
    let status_path = "target/rubash-help-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("help -x 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("help: -x: invalid option"));
    assert!(error.contains("help: usage: help [-dms] [pattern ...]"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_help_appends_stderr() {
    let _cwd_guard = CwdGuard::take();
    let error_path = "target/rubash-help-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("help -x 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("help: -x: invalid option"));
    assert!(error.contains("help: usage: help [-dms] [pattern ...]"));
    let _ = fs::remove_file(error_path);
}

/// rubash#3397: `dirs` prints the real working directory, never a
/// temporary `PWD=` assignment. GNU `dirs_builtin` takes the first
/// entry from `get_working_directory ("dirs")` and prints it through
/// `polite_directory_format` (builtins/pushd.def `dirs_builtin`;
/// general.c:953), which abbreviates a `$HOME` prefix to `~`. A `PWD=`
/// prefix only sets the variable for the command's environment — it
/// never chdirs — so it cannot steer `dirs`. The three tests below
/// keep the `PWD=` prefix as the nail and derive the expectation from
/// the live cwd (the embedded executor inherits the test process's
/// cwd and env — the env is snapshotted once at `Executor::new()`,
/// the cwd is read live from the process — executor/init.rs:326),
/// so the subject stays the redirect
/// plumbing, not the value. Call before `Executor::new()` so the
/// snapshot mirrors the executor's own.
fn expected_dirs_output() -> String {
    let cwd = std::env::current_dir().expect("test process cwd");
    let cwd = cwd.to_string_lossy().into_owned();
    let home = std::env::var("HOME").unwrap_or_default();
    if home.len() > 1 && (cwd == home || cwd.starts_with(&format!("{home}/"))) {
        format!("~{}\n", &cwd[home.len()..])
    } else {
        format!("{cwd}\n")
    }
}

#[test]
fn test_dirs_redirects_output() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-dirs-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("PWD=/tmp/rubash-dirs dirs > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let expected = expected_dirs_output();
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), expected);
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_dirs_appends_output() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-dirs-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("PWD=/tmp/rubash-dirs dirs >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let expected = expected_dirs_output();
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        format!("before\n{expected}")
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_dirs_redirects_output() {
    let _cwd_guard = CwdGuard::take();
    let output_path = "target/rubash-builtin-dirs-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("PWD=/tmp/rubash-builtin-dirs builtin dirs > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let expected = expected_dirs_output();
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), expected);
    let _ = fs::remove_file(output_path);
}
