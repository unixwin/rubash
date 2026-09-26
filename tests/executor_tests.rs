//! Executor Tests - TDD for Bash Command Executor
//!
//! Run with: cargo test --test executor_tests

use rubash::executor::{ExecuteError, Executor};
use rubash::lexer::tokenize;
use rubash::parser::parse;
use std::ffi::OsString;
use std::sync::Mutex;

pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn diagnostic_prefix_three_modes_match_gnu() {
    let _lock = ENV_LOCK.lock().unwrap();

    // Interactive mode: only shell name, no line segment
    let mut exec_interactive = Executor::new();
    exec_interactive.set_env("__RUBASH_INTERACTIVE", "1");
    exec_interactive.set_env("__RUBASH_SHELL_NAME", "niu");
    assert_eq!(exec_interactive.diagnostic_prefix(), "niu: ");

    // Script mode: script name + line (no INTERACTIVE flag)
    let mut exec_script = Executor::new();
    exec_script.unset_env("__RUBASH_INTERACTIVE");
    exec_script.set_env("__RUBASH_SCRIPT_NAME", "test.sh");
    exec_script.set_env("__RUBASH_CURRENT_LINE", "5");
    assert_eq!(exec_script.diagnostic_prefix(), "test.sh: line 5: ");

    // -c mode: bash + line (no script name, IS_C flag)
    let mut exec_c = Executor::new();
    exec_c.unset_env("__RUBASH_INTERACTIVE");
    exec_c.set_env("__RUBASH_CURRENT_LINE", "2");
    exec_c.set_env("__RUBASH_IS_C", "1");
    assert_eq!(exec_c.diagnostic_prefix(), "bash: line 2: ");

    // Fallback: no context (no line, no script, no interactive)
    let mut exec_fallback = Executor::new();
    exec_fallback.unset_env("__RUBASH_INTERACTIVE");
    exec_fallback.unset_env("__RUBASH_CURRENT_LINE");
    exec_fallback.unset_env("__RUBASH_SCRIPT_NAME");
    assert_eq!(exec_fallback.diagnostic_prefix(), "bash: ");
}

#[test]
fn builtin_error_prefix_three_modes_match_gnu() {
    let _lock = ENV_LOCK.lock().unwrap();

    // Interactive mode: only shell name, no line segment
    let mut env_interactive = std::collections::HashMap::new();
    env_interactive.insert("__RUBASH_INTERACTIVE".to_string(), "1".to_string());
    env_interactive.insert("__RUBASH_SHELL_NAME".to_string(), "niu".to_string());
    // Call through the public accessor in set module
    assert_eq!(
        rubash::builtins::set::builtin_error_prefix(&env_interactive),
        "niu: "
    );

    // Script mode: script name + line
    let mut env_script = std::collections::HashMap::new();
    env_script.insert("__RUBASH_SCRIPT_NAME".to_string(), "test.sh".to_string());
    env_script.insert("__RUBASH_CURRENT_LINE".to_string(), "5".to_string());
    assert_eq!(
        rubash::builtins::set::builtin_error_prefix(&env_script),
        "test.sh: line 5: "
    );

    // -c mode: rubash + line (no script name)
    let mut env_c = std::collections::HashMap::new();
    env_c.insert("__RUBASH_CURRENT_LINE".to_string(), "2".to_string());
    assert_eq!(
        rubash::builtins::set::builtin_error_prefix(&env_c),
        "rubash: line 2: "
    );

    // Fallback: no context
    let env_fallback = std::collections::HashMap::new();
    assert_eq!(
        rubash::builtins::set::builtin_error_prefix(&env_fallback),
        "rubash: "
    );
}

fn shell_test_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn shell_output_path_to_host(path: &str) -> std::path::PathBuf {
    if cfg!(windows) && path.len() >= 3 && path.as_bytes()[0] == b'/' && path.as_bytes()[2] == b'/'
    {
        let drive = path.as_bytes()[1] as char;
        return std::path::PathBuf::from(
            format!("{}:\\{}", drive.to_ascii_uppercase(), &path[3..]).replace('/', "\\"),
        );
    }
    std::path::PathBuf::from(path)
}

fn target_test_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(name)
}

fn write_executable(
    path: impl AsRef<std::path::Path>,
    contents: impl AsRef<[u8]>,
) -> std::io::Result<()> {
    std::fs::write(path.as_ref(), contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(path.as_ref())?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path.as_ref(), permissions)?;
    }
    Ok(())
}

fn test_command_path(bin_dir: &str, name: &str) -> String {
    if cfg!(windows) {
        format!("{bin_dir}/{name}.cmd")
    } else {
        format!("{bin_dir}/{name}")
    }
}

fn write_test_command(
    path: impl AsRef<std::path::Path>,
    unix_contents: impl AsRef<[u8]>,
    windows_contents: impl AsRef<[u8]>,
) -> std::io::Result<()> {
    if cfg!(windows) {
        write_executable(path, windows_contents)
    } else {
        write_executable(path, unix_contents)
    }
}

fn read_normalized(path: impl AsRef<std::path::Path>) -> String {
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

mod simple_execution {
    use super::*;

    #[test]
    fn test_echo_command() {
        let input = "echo hello";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_whitespace_braced_substitution_is_bad_substitution() {
        // GNU Bash 5.2.21 (subst.c param_expand bad_substitution): a
        // whitespace-led `${ command; }` word is a non-fatal word-expansion
        // error - it abandons the current command with status 1 and the
        // next line still runs.
        let tokens = tokenize("echo ${ printf x; }");
        let ast = parse(&tokens);
        let mut executor = Executor::new();

        let result = executor.execute_ast(&ast);

        assert!(matches!(result, Err(ExecuteError::ExpansionFailure(1))));
        assert_eq!(executor.last_exit_code(), 1);
    }

    #[test]
    fn test_echo_multiple_args() {
        let input = "echo hello world";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_exit_command() {
        let input = "exit 0";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_err());
    }

    #[test]
    fn test_pwd_command() {
        let input = "pwd";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_true_command() {
        let tokens = tokenize("true");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).ok();
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn test_false_command() {
        let tokens = tokenize("false");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).ok();
        assert_eq!(executor.last_exit_code(), 1);
    }
}

mod exit_codes {
    use super::*;

    #[test]
    fn test_exit_with_code() {
        let input = "exit 42";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).unwrap_err();
        assert_eq!(executor.last_exit_code(), 42);
    }

    #[test]
    fn test_exit_without_code() {
        let input = "exit";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).unwrap_err();
        assert_eq!(executor.last_exit_code(), 0);
    }
}

mod environment_tests {
    use super::*;

    #[test]
    fn test_export_command() {
        let input = "export TEST_VAR=hello";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_env_var_storage() {
        let mut executor = Executor::new();
        executor.set_env("MY_VAR", "hello");
        assert_eq!(executor.get_env("MY_VAR"), Some("hello"));
    }

    #[test]
    fn public_export_env_marks_variable_for_child_environment() {
        let mut executor = Executor::new();
        executor.export_env("RUBASH_PUBLIC_EXPORT_ENV", "visible");
        let snapshot = executor.env_vars_snapshot();

        assert_eq!(
            snapshot.get("RUBASH_PUBLIC_EXPORT_ENV"),
            Some(&"visible".to_string())
        );
        assert!(snapshot
            .get("__RUBASH_EXPORTED_VARS")
            .is_some_and(|value| value
                .split('\x1f')
                .any(|name| name == "RUBASH_PUBLIC_EXPORT_ENV")));
    }

    #[test]
    fn default_shell_is_current_executable_when_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _shell = EnvGuard::unset("SHELL");
        let _bash = EnvGuard::unset("BASH");

        let executor = Executor::new();

        assert_eq!(executor.get_env("SHELL"), executor.get_env("BASH"));
        assert!(executor
            .get_env("SHELL")
            .is_some_and(|value| !value.is_empty()));
    }

    #[test]
    fn inherited_shell_is_preserved() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _shell = EnvGuard::set("SHELL", "C:/custom/shell.exe");

        let executor = Executor::new();

        assert_eq!(executor.get_env("SHELL"), Some("C:/custom/shell.exe"));
    }

    #[test]
    fn test_unset_command() {
        let input = "unset HOME";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    struct EnvGuard {
        name: &'static str,
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(name);
            std::env::set_var(name, value);
            Self { name, previous }
        }

        fn unset(name: &'static str) -> Self {
            let previous = std::env::var_os(name);
            std::env::remove_var(name);
            Self { name, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.name, previous);
            } else {
                std::env::remove_var(self.name);
            }
        }
    }
}

mod function_api_tests {
    use super::*;

    #[test]
    fn public_call_function_invokes_defined_function_with_temporary_env() {
        let output_path = target_test_path("rubash-public-function-call-output.txt");
        let shell_output_path = shell_test_path(&output_path);
        let _ = std::fs::remove_file(&output_path);
        let input = format!(
            "hook() {{ printf '%s:%s:%s\\n' \"$HOOK_CTX\" \"$1\" \"$#\" > {shell_output_path}; HOOK_CTX=changed; HOOK_SIDE=kept; return 7; }}"
        );
        let tokens = tokenize(&input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("HOOK_CTX", "base");
        executor.execute_ast(&ast).unwrap();

        assert!(executor.has_function("hook"));
        let status = executor
            .call_function_with_env("hook", ["alpha"], [("HOOK_CTX", "temp")])
            .unwrap();

        assert_eq!(status, 7);
        assert_eq!(executor.last_exit_code(), 7);
        assert_eq!(executor.get_env("HOOK_CTX"), Some("base"));
        assert_eq!(executor.get_env("HOOK_SIDE"), Some("kept"));
        assert_eq!(read_normalized(&output_path), "temp:alpha:1\n");
        let _ = std::fs::remove_file(output_path);
    }

    #[test]
    fn public_call_function_reports_missing_function_without_path_lookup() {
        let mut executor = Executor::new();

        let error = executor
            .call_function("missing_hook", std::iter::empty::<&str>())
            .unwrap_err();

        assert!(!executor.has_function("missing_hook"));
        assert!(matches!(
            error,
            ExecuteError::FunctionNotFound(name) if name == "missing_hook"
        ));
    }

    #[test]
    fn functions_snapshot_returns_defined_function_names_sorted() {
        let _env_guard = ENV_LOCK.lock().unwrap();
        let imported_function_var = "BASH_FUNC_rubash_imported%%";
        let old_imported_function = std::env::var_os(imported_function_var);
        std::env::remove_var(imported_function_var);
        let tokens = tokenize("zeta() { :; }; alpha() { :; }");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        match old_imported_function {
            Some(value) => std::env::set_var(imported_function_var, value),
            None => std::env::remove_var(imported_function_var),
        }
        executor.execute_ast(&ast).unwrap();

        assert_eq!(
            executor.functions_snapshot(),
            vec!["alpha".to_string(), "zeta".to_string()]
        );
    }
}

#[cfg(windows)]
mod windows_script_commands {
    use super::*;

    #[test]
    fn dot_sh_path_command_runs_in_rubash_without_leaking_state() {
        let dir = target_test_path("windows-direct-sh-script");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("probe.sh");
        let marker = dir.join("marker.txt");
        write_executable(
            &script,
            format!(
                "echo \"$__RUBASH_SCRIPT_NAME|$1|$#|$BASH_SUBSHELL\" > {}\ncd ..\nexport RUBASH_DIRECT_SCRIPT_LEAK=1\n",
                shell_test_path(&marker)
            ),
        )
        .unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();

        let tokens = tokenize("./probe.sh one two");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.set_env("__RUBASH_SCRIPT_NAME", "winuxsh");
        executor.execute_ast(&ast).unwrap();
        let current_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(old_cwd).unwrap();

        assert_eq!(
            std::fs::read_to_string(&marker).unwrap().trim(),
            "./probe.sh|one|2|1"
        );
        assert_eq!(executor.get_env("RUBASH_DIRECT_SCRIPT_LEAK"), None);
        assert_eq!(current_cwd, dir);
    }

    #[test]
    fn dot_ps1_path_command_runs_with_powershell() {
        if !powershell_runtime_available() {
            return;
        }

        let dir = target_test_path("windows-direct-ps1-script");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("probe.ps1");
        let marker = dir.join("marker.txt");
        let marker_literal = marker.to_string_lossy().replace('\'', "''");
        write_executable(
            &script,
            format!("Set-Content -Path '{marker_literal}' -Value ('ps1:' + ($args -join ','))\n"),
        )
        .unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();

        let tokens = tokenize("./probe.ps1 one two");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).unwrap();
        let status = executor.last_exit_code();
        std::env::set_current_dir(old_cwd).unwrap();

        assert_eq!(status, 0);
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap().trim(),
            "ps1:one,two"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    fn powershell_runtime_available() -> bool {
        ["pwsh.exe", "powershell.exe"].into_iter().any(|name| {
            std::process::Command::new("where.exe")
                .arg(name)
                .output()
                .is_ok_and(|output| output.status.success())
        })
    }
}

#[path = "executor_command_chaining/mod.rs"]
mod command_chaining;

mod builtin_commands {
    use super::*;

    #[test]
    fn test_env_builtin() {
        let input = "env";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_set_builtin() {
        let input = "set";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_test_builtin() {
        let input = "test 1 -eq 1";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }

    #[test]
    fn test_bracket_builtin() {
        let input = "[ 1 -eq 1 ]";
        let tokens = tokenize(input);
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_ok());
    }
}
