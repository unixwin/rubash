use super::super::*;
use std::fs;

#[test]
fn test_hash_p_requires_pathname_argument() {
    let error_path = "target/rubash-hash-p-missing-stderr-output.txt";
    let status_path = "target/rubash-hash-p-missing-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("hash -p 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("hash: -p: option requires an argument"));
    assert!(error.contains("hash: usage:"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_hash_p_without_name_prints_table() {
    let output_path = "target/rubash-hash-p-without-name-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("hash -p /tmp/rubash-cat cat; hash -p /tmp/ignored > {output_path}; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("hits\tcommand\n"));
    assert!(output.contains("/tmp/rubash-cat\n"));
    assert!(output.ends_with("0\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_hash_p_compact_pathname_sets_entry() {
    let output_path = "target/rubash-hash-p-compact-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("hash -p/tmp/rubash-compact-cat cat; hash -t cat > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "/tmp/rubash-compact-cat\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_hash_redirects_stderr() {
    let error_path = "target/rubash-hash-stderr-output.txt";
    let status_path = "target/rubash-hash-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("hash -t no_such_command 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "1\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("hash: no_such_command: not found"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_hash_appends_stderr() {
    let error_path = "target/rubash-hash-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("hash -t no_such_command 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("hash: no_such_command: not found"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_shopt_redirects_output() {
    let output_path = "target/rubash-shopt-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("shopt -p sourcepath > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "shopt -s sourcepath\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shopt_appends_output() {
    let output_path = "target/rubash-shopt-append-output.txt";
    let _ = fs::remove_file(output_path);
    fs::write(output_path, "before\n").unwrap();
    let input = format!("shopt -p sourcepath >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "before\nshopt -s sourcepath\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_shopt_updates_shell_option_state() {
    let output_path = "target/rubash-builtin-shopt-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("builtin shopt -s nullglob; shopt -q nullglob; echo $? > {output_path}");
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
fn test_shopt_print_mode_preserves_query_status() {
    let output_path = "target/rubash-shopt-print-status.txt";
    let _ = fs::remove_file(output_path);
    let input = format!(
        "shopt -u nullglob; shopt -p nullglob > {output_path}; echo $? >> {output_path}; \
         shopt -s nullglob; shopt -p nullglob >> {output_path}; echo $? >> {output_path}; \
         shopt -o -p errexit >> {output_path}; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "shopt -u nullglob\n1\nshopt -s nullglob\n0\nset +o errexit\n1\n"
    );
    let _ = fs::remove_file(output_path);
}

// rubash#393 resolved: `shopt -o` listings print with shopt.def:73 OPTFMT
// "%-20s\t%s\n" (NOT set.def:291's 15-wide MINUS_O_FORMAT) — verified
// byte-identical against WSL GNU 5.3.0 (target/gapfix2/sho.sh: both print
// "pipefail" + 12 spaces + TAB + "off"). The pre-existing engine output
// was already conformant; the wt33 expectation (15-wide) was wrong for
// GNU. `set -o` keeps the 15-wide form (set.def:291, probe so2.sh).
#[test]
fn test_shopt_o_uses_readable_format_without_print_flag() {
    let output_path = "target/rubash-shopt-o-readable-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("shopt -o pipefail > {output_path}; shopt -o -p pipefail >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    assert!(executor.execute_ast(&ast).is_ok());
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "pipefail            \toff\nset +o pipefail\n"
    );
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shopt_list_returns_failure_for_disabled_option() {
    let output_path = "target/rubash-shopt-list-disabled-status.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("shopt -u expand_aliases; shopt expand_aliases >/dev/null; echo $? > {output_path}; shopt -s expand_aliases; shopt expand_aliases >/dev/null; echo $? >> {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(output_path).unwrap(), "1\n0\n");
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_shopt_redirects_stderr() {
    let error_path = "target/rubash-shopt-stderr-output.txt";
    let status_path = "target/rubash-shopt-stderr-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("shopt -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("shopt: -Z: invalid option"));
    assert!(error.contains("shopt: usage:"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_shopt_appends_stderr() {
    let error_path = "target/rubash-shopt-stderr-append-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("shopt -Z 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 2);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("shopt: -Z: invalid option"));
    assert!(error.contains("shopt: usage:"));
    let _ = fs::remove_file(error_path);
}

// rubash#392 fixed: cd's redirect binding now funnels through
// sync_cd_variables for every shape, so a cdable_vars `cd dest > f`
// updates PWD/OLDPWD in the live shell (GNU builtins/cd.def:136-175
// bindpwd runs in the current process for builtins; probe
// wt33-373/run/H04_cdable_vars_pwd).
#[test]
fn test_cdable_vars_uses_variable_as_directory() {
    let original_dir = std::env::current_dir().unwrap();
    let original_pwd = std::env::var("PWD").ok();
    let original_oldpwd = std::env::var("OLDPWD").ok();
    let root = original_dir.join("target/rubash-cdable-vars");
    let dest_dir = root.join("dest");
    let output_path = root.join("output.txt");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&dest_dir).unwrap();

    // Author the variable value in the POSIX /<drive>/ domain — the same
    // single-domain shape as the WSL GNU probe — so the two lines match:
    // line 1 is GNU cd.def:397-400's VERBATIM variable-value print, line 2
    // is $PWD in rubash's canonical POSIX style (see the pwd_value gate in
    // builtins/cd.rs; a D:/ value would legitimately print D:/ verbatim
    // on line 1 while $PWD stays /d/).
    let dest_win = shell_test_path(&dest_dir);
    let mut drive_chars = dest_win.chars();
    let drive = drive_chars.next().unwrap_or('d').to_ascii_lowercase();
    let rest: String = drive_chars.collect();
    let dest_display = format!("/{drive}{}", rest.trim_start_matches(':'));
    let output_display = output_path.to_string_lossy().replace('\\', "/");
    let input = format!(
        "shopt -s cdable_vars; dest='{dest_display}'; cd dest > {output_display}; echo $PWD >> {output_display}"
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
        format!("{dest_display}\n{dest_display}\n")
    );
    let _ = fs::remove_dir_all(&root);
}
