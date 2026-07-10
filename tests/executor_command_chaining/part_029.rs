use super::super::*;
use std::fs;

#[test]
fn test_unset_nameref_removes_target_unless_n_option_is_used() {
    let output_path = target_test_path("rubash-unset-nameref-output.txt");
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_file(&output_path);
    for name in [
        "RUBASH_UNSET_NAMEREF_TARGET",
        "RUBASH_UNSET_NAMEREF_REF",
        "RUBASH_UNSET_NAMEREF_TARGET2",
        "RUBASH_UNSET_NAMEREF_REF2",
    ] {
        std::env::remove_var(name);
    }
    let input = format!(
        "RUBASH_UNSET_NAMEREF_TARGET=value; declare -n RUBASH_UNSET_NAMEREF_REF=RUBASH_UNSET_NAMEREF_TARGET; \
         unset RUBASH_UNSET_NAMEREF_REF; \
         declare -p RUBASH_UNSET_NAMEREF_TARGET 2>/dev/null || echo target-unset > {shell_output_path}; \
         declare -p RUBASH_UNSET_NAMEREF_REF >> {shell_output_path}; \
         RUBASH_UNSET_NAMEREF_TARGET2=value; declare -n RUBASH_UNSET_NAMEREF_REF2=RUBASH_UNSET_NAMEREF_TARGET2; \
         unset -n RUBASH_UNSET_NAMEREF_REF2; \
         declare -p RUBASH_UNSET_NAMEREF_TARGET2 >> {shell_output_path}; \
         declare -p RUBASH_UNSET_NAMEREF_REF2 2>/dev/null || echo ref-unset >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "target-unset\ndeclare -n RUBASH_UNSET_NAMEREF_REF=\"RUBASH_UNSET_NAMEREF_TARGET\"\ndeclare -- RUBASH_UNSET_NAMEREF_TARGET2=\"value\"\nref-unset\n"
    );
    for name in [
        "RUBASH_UNSET_NAMEREF_TARGET",
        "RUBASH_UNSET_NAMEREF_REF",
        "RUBASH_UNSET_NAMEREF_TARGET2",
        "RUBASH_UNSET_NAMEREF_REF2",
    ] {
        std::env::remove_var(name);
    }
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_unset_nameref_target_reports_readonly_target() {
    let output_path = target_test_path("rubash-unset-nameref-readonly-output.txt");
    let error_path = target_test_path("rubash-unset-nameref-readonly-error.txt");
    let shell_output_path = shell_test_path(&output_path);
    let shell_error_path = shell_test_path(&error_path);
    let _ = fs::remove_file(&output_path);
    let _ = fs::remove_file(&error_path);
    std::env::remove_var("RUBASH_UNSET_NAMEREF_RO_TARGET");
    std::env::remove_var("RUBASH_UNSET_NAMEREF_RO_REF");
    let input = format!(
        "readonly RUBASH_UNSET_NAMEREF_RO_TARGET=value; \
         declare -n RUBASH_UNSET_NAMEREF_RO_REF=RUBASH_UNSET_NAMEREF_RO_TARGET; \
         unset RUBASH_UNSET_NAMEREF_RO_REF 2> {shell_error_path}; echo $? > {shell_output_path}; \
         declare -p RUBASH_UNSET_NAMEREF_RO_TARGET RUBASH_UNSET_NAMEREF_RO_REF >> {shell_output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "1\ndeclare -r RUBASH_UNSET_NAMEREF_RO_TARGET=\"value\"\ndeclare -n RUBASH_UNSET_NAMEREF_RO_REF=\"RUBASH_UNSET_NAMEREF_RO_TARGET\"\n"
    );
    let error = fs::read_to_string(&error_path).unwrap();
    assert!(
        error.contains("unset: RUBASH_UNSET_NAMEREF_RO_TARGET: cannot unset: readonly variable")
    );
    std::env::remove_var("RUBASH_UNSET_NAMEREF_RO_TARGET");
    std::env::remove_var("RUBASH_UNSET_NAMEREF_RO_REF");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_unset_invalid_option_redirects_stderr() {
    let error_path = "target/rubash-unset-invalid-option-stderr-output.txt";
    let status_path = "target/rubash-unset-invalid-option-status.txt";
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
    let input = format!("unset -Z 2> {error_path}; echo $? > {status_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(status_path).unwrap(), "2\n");
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.contains("unset: -Z: invalid option"));
    assert!(error.contains("unset: usage:"));
    let _ = fs::remove_file(error_path);
    let _ = fs::remove_file(status_path);
}

#[test]
fn test_unset_conflicting_options_append_stderr() {
    let error_path = "target/rubash-unset-conflicting-options-stderr-output.txt";
    let _ = fs::remove_file(error_path);
    fs::write(error_path, "before\n").unwrap();
    let input = format!("unset -fv value 2>> {error_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 1);
    let error = fs::read_to_string(error_path).unwrap();
    assert!(error.starts_with("before\n"));
    assert!(error.contains("unset: cannot simultaneously unset a function and a variable"));
    let _ = fs::remove_file(error_path);
}

#[test]
fn test_enable_redirects_output() {
    let output_path = "target/rubash-enable-redirect-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("enable -ps > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("enable break\n"));
    assert!(output.contains("enable times\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_enable_p_lists_enabled_builtins() {
    let output_path = "target/rubash-enable-p-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("enable -p > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("enable echo\n"));
    assert!(output.contains("enable test\n"));
    assert!(output.contains("enable source\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_enable_a_marks_disabled_builtins() {
    let output_path = "target/rubash-enable-a-output.txt";
    let _ = fs::remove_file(output_path);
    let input = format!("enable -n echo; enable -a > {output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    let output = fs::read_to_string(output_path).unwrap();
    assert!(output.contains("enable -n echo\n"));
    assert!(output.contains("enable test\n"));
    let _ = fs::remove_file(output_path);
}

#[test]
fn test_builtin_enable_updates_disabled_builtin_state() {
    let bin_dir = target_test_path("rubash-builtin-enable-bin");
    #[cfg(windows)]
    let script_path = bin_dir.join("test.cmd");
    #[cfg(not(windows))]
    let script_path = bin_dir.join("test");
    let output_path = target_test_path("rubash-builtin-enable-output.txt");
    let shell_bin_dir = shell_test_path(&bin_dir);
    let shell_output_path = shell_test_path(&output_path);
    let _ = fs::remove_dir_all(&bin_dir);
    let _ = fs::remove_file(&output_path);
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(&script_path, "echo external-test\n").unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
    }
    let input = format!("builtin enable -n test; type -t test > {shell_output_path}");
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    executor.set_env("PATH", &shell_bin_dir);

    let result = executor.execute_ast(&ast);

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "file\n");
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir_all(bin_dir);
}

#[test]
fn test_disabled_test_builtin_uses_external_command() {
    let bin_dir = "target/rubash-disabled-test-bin";
    let script_path = format!("{bin_dir}/test");
    let output_path = "target/rubash-disabled-test-output.txt";
    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_file(output_path);
    fs::create_dir_all(bin_dir).unwrap();
    fs::write(&script_path, "echo external-test\n").unwrap();
    let input = format!(
        "enable -n test; test > {output_path}; enable test; test 1 -eq 1; echo $? >> {output_path}"
    );
    let tokens = tokenize(&input);
    let ast = parse(&tokens);
    let mut executor = Executor::new();
    let old_path = std::env::var("PATH").ok();
    executor.set_env("PATH", bin_dir);

    let result = executor.execute_ast(&ast);
    match old_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }

    assert!(result.is_ok());
    assert_eq!(executor.last_exit_code(), 0);
    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "external-test\n0\n"
    );
    let _ = fs::remove_file(script_path);
    let _ = fs::remove_file(output_path);
    let _ = fs::remove_dir(bin_dir);
}
